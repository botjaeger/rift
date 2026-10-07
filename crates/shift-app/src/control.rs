//! Local control API: each Shift window listens on
//! `<data dir>/control/<pid>.sock` (a 0700 directory; other uids are
//! refused) for newline-delimited JSON requests, one response line each.
//!
//! `control_api` in config.toml gates what is accepted:
//!
//! | level | commands |
//! |---|---|
//! | `"off"` | no socket |
//! | `"navigate"` (default) | `list`, `status`, `focus` |
//! | `"full"` | also `split`, `open`, `send-text` (types into a shell) |
//!
//! `shift-terminal ctl …` is the client. Requests are answered on the UI
//! thread, so a response reflects exactly what the window shows.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off,
    Navigate,
    Full,
}

impl Level {
    pub fn parse(s: &str) -> Level {
        match s.trim() {
            "off" => Level::Off,
            "full" => Level::Full,
            _ => Level::Navigate,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    List,
    Status,
    Focus {
        #[serde(default)]
        tab: Option<usize>,
        #[serde(default)]
        pane: Option<u64>,
    },
    Split {
        /// "right" or "down".
        direction: String,
        #[serde(default)]
        cwd: Option<String>,
    },
    Open {
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        command: Option<String>,
    },
    SendText {
        pane: u64,
        text: String,
    },
}

impl Request {
    /// Least level that may run this request.
    pub fn level(&self) -> Level {
        match self {
            Request::List | Request::Status | Request::Focus { .. } => Level::Navigate,
            _ => Level::Full,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub id: u64,
    pub active: bool,
    pub cwd: Option<String>,
    /// Foreground job other than the shell.
    pub job: Option<String>,
    pub last_command: Option<String>,
    pub last_exit: Option<i32>,
    pub durable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabInfo {
    pub index: usize,
    pub title: String,
    pub active: bool,
    pub panes: Vec<PaneInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<TabInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<u64>,
}

impl Response {
    pub fn error(msg: impl Into<String>) -> Response {
        Response {
            ok: false,
            error: Some(msg.into()),
            ..Default::default()
        }
    }
}

/// A request waiting for the UI thread.
#[derive(Debug)]
pub struct Pending {
    pub request: Request,
    pub reply: mpsc::Sender<Response>,
    active: Arc<AtomicBool>,
}

impl Pending {
    pub fn allowed(&self, level: Level) -> bool {
        self.active.load(Ordering::Acquire) && self.request.level() <= level
    }
}

pub fn dir() -> Option<PathBuf> {
    shift_pty::ipc::socket_dir("control")
}

fn own_path() -> Option<PathBuf> {
    dir().map(|d| d.join(format!("{}.sock", std::process::id())))
}

/// Start listening; every request is handed to `deliver` (which forwards
/// it to the UI thread). The socket file is removed by [`Server`]'s drop.
pub struct Server {
    path: PathBuf,
    pub level: Level,
    active: Arc<AtomicBool>,
    connections: Arc<Mutex<Vec<Weak<UnixStream>>>>,
    listener: Option<std::thread::JoinHandle<()>>,
    wake: UnixStream,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        let _ = self.wake.shutdown(std::net::Shutdown::Both);
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        for stream in self
            .connections
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
        {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn serve(level: Level, deliver: impl Fn(Pending) + Send + Sync + 'static) -> Option<Server> {
    if level == Level::Off {
        return None;
    }
    let dir = dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    // sockets of windows that are gone
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if UnixStream::connect(&p).is_err() {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    let path = own_path()?;
    let _ = std::fs::remove_file(&path);
    start(path, level, deliver)
}

const MAX_CONNECTIONS: usize = 32;
const MAX_REQUEST: usize = 1 << 20;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

fn start(
    path: PathBuf,
    level: Level,
    deliver: impl Fn(Pending) + Send + Sync + 'static,
) -> Option<Server> {
    let listener = UnixListener::bind(&path).ok()?;
    listener.set_nonblocking(true).ok()?;
    let (wake, stop) = UnixStream::pair().ok()?;
    let deliver = Arc::new(deliver);
    let active = Arc::new(AtomicBool::new(true));
    let connections = Arc::new(Mutex::new(Vec::<Weak<UnixStream>>::new()));
    let running = active.clone();
    let clients = connections.clone();
    let listener = std::thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            let me = unsafe { libc::getuid() };
            while running.load(Ordering::Acquire) {
                let mut ready = [
                    libc::pollfd {
                        fd: listener.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: stop.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                ];
                let result = unsafe { libc::poll(ready.as_mut_ptr(), 2, -1) };
                if !running.load(Ordering::Acquire) {
                    break;
                }
                if result < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(_) => break,
                };
                if shift_pty::ipc::peer_uid(&stream) != Some(me) {
                    continue;
                }
                let stream = Arc::new(stream);
                let mut clients = clients.lock().unwrap();
                clients.retain(|c| c.strong_count() > 0);
                if clients.len() >= MAX_CONNECTIONS {
                    continue;
                }
                clients.push(Arc::downgrade(&stream));
                let deliver = deliver.clone();
                let active = running.clone();
                std::thread::spawn(move || handle_conn(stream, level, active, &*deliver));
            }
        })
        .ok()?;
    Some(Server {
        path,
        level,
        active,
        connections,
        listener: Some(listener),
        wake,
    })
}

/// `take` bounds allocation even when no newline ever arrives.
fn read_line_bounded(reader: &mut impl BufRead, limit: usize) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let n = reader.take(limit as u64 + 1).read_line(&mut line)?;
    if n > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "control line too large",
        ));
    }
    Ok((n != 0).then_some(line))
}

fn handle_conn(
    stream: Arc<UnixStream>,
    level: Level,
    active: Arc<AtomicBool>,
    deliver: &dyn Fn(Pending),
) {
    if stream.set_read_timeout(Some(IO_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(IO_TIMEOUT)).is_err()
    {
        return;
    }
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(&*stream);
    while active.load(Ordering::Acquire) {
        let Ok(Some(line)) = read_line_bounded(&mut reader, MAX_REQUEST) else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response::error(format!("bad request: {e}")),
            Ok(req) if req.level() > level => Response::error(format!(
                "not allowed: set control_api = \"full\" in config.toml to use {:?}",
                req
            )),
            Ok(request) => {
                let (tx, rx) = mpsc::channel();
                if !active.load(Ordering::Acquire) {
                    break;
                }
                deliver(Pending {
                    request,
                    reply: tx,
                    active: active.clone(),
                });
                rx.recv_timeout(Duration::from_secs(5))
                    .unwrap_or_else(|_| Response::error("window did not answer"))
            }
        };
        let Ok(mut body) = serde_json::to_string(&response) else {
            break;
        };
        body.push('\n');
        if out.write_all(body.as_bytes()).is_err() {
            break;
        }
    }
}

/// Every window's socket, newest first.
pub fn sockets() -> Vec<PathBuf> {
    let Some(dir) = dir() else { return Vec::new() };
    let mut socks: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "sock"))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect()
        })
        .unwrap_or_default();
    socks.sort_by_key(|a| std::cmp::Reverse(a.0));
    socks.into_iter().map(|(_, p)| p).collect()
}

/// One request to one window's socket.
pub fn request(path: &std::path::Path, req: &Request) -> std::io::Result<Response> {
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(Duration::from_secs(6)))?;
    s.set_write_timeout(Some(Duration::from_secs(6)))?;
    let mut body = serde_json::to_string(req).map_err(std::io::Error::other)?;
    body.push('\n');
    s.write_all(body.as_bytes())?;
    let line = read_line_bounded(&mut BufReader::new(s), 16 << 20)?
        .ok_or_else(|| std::io::Error::other("window disconnected"))?;
    serde_json::from_str(&line).map_err(std::io::Error::other)
}

/// Other windows' tabs (for the session switcher): (pid, socket, tabs).
pub fn visit_other_windows(mut found: impl FnMut((u32, PathBuf, Vec<TabInfo>))) {
    let me = std::process::id();
    for window in sockets().into_iter().filter_map(|p| {
        let pid: u32 = p.file_stem()?.to_str()?.parse().ok()?;
        if pid == me {
            return None;
        }
        let r = request(&p, &Request::List).ok()?;
        Some((pid, p, r.tabs))
    }) {
        found(window);
    }
}

const CLI_HELP: &str = "usage: shift-terminal ctl [--pid PID] <command>

  list                          tabs and panes of every window (JSON)
  status                        this window's pid, version, tabs
  focus [--tab N] [--pane ID]   raise the window, select a tab or pane
  split right|down [--cwd DIR]  split the active pane        (control_api = \"full\")
  open [--cwd DIR] [-- CMD…]    new tab, optionally running CMD (\"full\")
  send-text --pane ID TEXT      type TEXT into a pane         (\"full\")

The newest window answers unless --pid names one.";

/// `shift-terminal ctl …`. Returns the process exit code.
pub fn cli(args: &[String]) -> i32 {
    let mut args: Vec<String> = args.to_vec();
    let mut pid: Option<String> = None;
    if let Some(i) = args.iter().position(|a| a == "--pid") {
        pid = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
    }
    let flag = |args: &mut Vec<String>, name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        let v = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
        v
    };
    let Some(cmd) = args.first().cloned() else {
        println!("{CLI_HELP}");
        return 2;
    };
    let mut rest: Vec<String> = args[1..].to_vec();
    let req = match cmd.as_str() {
        "list" => {
            // every window
            let mut all = Vec::new();
            for p in sockets() {
                if let Ok(r) = request(&p, &Request::Status) {
                    all.push(r);
                }
            }
            println!("{}", serde_json::to_string_pretty(&all).unwrap_or_default());
            return 0;
        }
        "status" => Request::Status,
        "focus" => Request::Focus {
            tab: flag(&mut rest, "--tab").and_then(|v| v.parse().ok()),
            pane: flag(&mut rest, "--pane").and_then(|v| v.parse().ok()),
        },
        "split" => Request::Split {
            cwd: flag(&mut rest, "--cwd"),
            direction: rest.first().cloned().unwrap_or_else(|| "right".into()),
        },
        "open" => {
            let command = rest
                .iter()
                .position(|a| a == "--")
                .map(|i| rest[i + 1..].join(" "))
                .filter(|c| !c.is_empty());
            Request::Open {
                cwd: flag(&mut rest, "--cwd"),
                command,
            }
        }
        "send-text" => {
            let Some(pane) = flag(&mut rest, "--pane").and_then(|v| v.parse().ok()) else {
                eprintln!("send-text needs --pane ID");
                return 2;
            };
            Request::SendText {
                pane,
                text: rest.join(" "),
            }
        }
        _ => {
            println!("{CLI_HELP}");
            return 2;
        }
    };
    let target = match &pid {
        Some(p) => dir().map(|d| d.join(format!("{p}.sock"))),
        None => sockets().into_iter().next(),
    };
    let Some(target) = target else {
        eprintln!("no running Shift window (or control_api = \"off\")");
        return 1;
    };
    match request(&target, &req) {
        Ok(r) => {
            println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            if r.ok { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("{}: {e}", target.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unterminated_requests_stop_at_the_limit() {
        let mut infinite = BufReader::new(std::io::repeat(b'x'));
        assert_eq!(
            read_line_bounded(&mut infinite, MAX_REQUEST)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
        let mut lines = std::io::Cursor::new(b"one\ntwo\n");
        assert_eq!(
            read_line_bounded(&mut lines, 4).unwrap().as_deref(),
            Some("one\n")
        );
        assert_eq!(
            read_line_bounded(&mut lines, 4).unwrap().as_deref(),
            Some("two\n")
        );
        assert!(read_line_bounded(&mut lines, 4).unwrap().is_none());
    }

    #[test]
    fn dropping_server_revokes_queued_work_and_open_connections() {
        let path = std::env::temp_dir().join(format!("shift-revoke-{}.sock", std::process::id()));
        let (tx, rx) = mpsc::channel();
        let server = start(path.clone(), Level::Full, move |p| {
            let _ = tx.send(p);
        })
        .unwrap();
        let mut peer = UnixStream::connect(&path).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.write_all(b"{\"cmd\":\"send-text\",\"pane\":1,\"text\":\"x\"}\n")
            .unwrap();
        let pending = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(pending.allowed(Level::Full));
        assert!(!pending.allowed(Level::Navigate));
        drop(server);
        assert!(!pending.allowed(Level::Full));
        assert!(!path.exists());
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        let _ = pending.reply.send(Response::error("revoked"));
        let server = start(path.clone(), Level::Navigate, |p| {
            let _ = p.reply.send(Response {
                ok: true,
                ..Default::default()
            });
        })
        .unwrap();
        assert!(request(&path, &Request::Status).unwrap().ok);
        assert!(
            !pending.allowed(Level::Full),
            "rebinding cannot revive old work"
        );
        drop(server);
    }

    #[test]
    fn requests_parse_and_levels_gate() {
        let r: Request = serde_json::from_str(r#"{"cmd":"focus","pane":3}"#).unwrap();
        assert_eq!(
            r,
            Request::Focus {
                tab: None,
                pane: Some(3)
            }
        );
        assert_eq!(r.level(), Level::Navigate);
        let r: Request =
            serde_json::from_str(r#"{"cmd":"send-text","pane":1,"text":"ls\n"}"#).unwrap();
        assert_eq!(r.level(), Level::Full);
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"rm-rf"}"#).is_err());
        assert_eq!(Level::parse("full"), Level::Full);
        assert_eq!(Level::parse("nonsense"), Level::Navigate);
    }

    #[test]
    fn socket_round_trip_and_permission_gate() {
        let dir = std::env::temp_dir().join(format!("shift-ctl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            handle_conn(
                Arc::new(s),
                Level::Navigate,
                Arc::new(AtomicBool::new(true)),
                &|p: Pending| {
                    let _ = p.reply.send(Response {
                        ok: true,
                        pid: Some(42),
                        ..Default::default()
                    });
                },
            );
        });
        let mut s = UnixStream::connect(&path).unwrap();
        s.write_all(
            b"{\"cmd\":\"status\"}\n{\"cmd\":\"send-text\",\"pane\":1,\"text\":\"x\"}\nnot json\n",
        )
        .unwrap();
        let mut lines = BufReader::new(s).lines();
        let ok: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(ok.ok && ok.pid == Some(42));
        let denied: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(!denied.ok && denied.error.unwrap().contains("not allowed"));
        let bad: Response = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(bad.error.unwrap().starts_with("bad request"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
