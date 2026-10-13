//! `rift-terminal remote`: the gateway for the mobile companion
//! (`rift_remote`, protocol v2), fed by the windows' control sockets
//! (`control.rs`), and its pairing / device commands.
//!
//! Topology is each window's `status`; blocks are the owning window's
//! `blocks` request; `run` / `keys` from operate devices become the owning
//! window's `remote-input` request, which the window accepts only while
//! `remote_access` is on.

use crate::control::{self, Request, Response};
use rift_remote::protocol::{Pane, Tab, Window};
use rift_remote::registry::{self, Permission};
use rift_remote::{Input, InputError, Page, Source, SourceError};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_LISTEN: &str = "0.0.0.0:7878";
/// `--managed`: how often the config and the windows are checked.
const MANAGED_CHECK: Duration = Duration::from_millis(500);
/// `--managed`: exit this long after the last window is gone.
const NO_WINDOWS_EXIT: Duration = Duration::from_secs(30);

const HELP: &str = "usage: rift-terminal remote [--listen ADDR] [--managed]
       rift-terminal remote pair [--view]
       rift-terminal remote devices
       rift-terminal remote revoke ID
       rift-terminal remote permission ID operate|view
       rift-terminal remote history [--limit N]

Serve this user's Rift windows to paired phones (Noise-encrypted WebSocket,
protocol v2). ADDR defaults to `remote_listen` in config.toml, else
0.0.0.0:7878. With `remote_relay` set, the gateway also registers at that
relay (followed while it runs). One gateway runs per user.

  --managed      started by a Rift window: exits within 2 s after
                 `remote_access` becomes false, and 30 s after the last
                 window is gone
  pair [--view]  print a one-time pairing QR code and link (valid 2 minutes);
                 --view pairs a device that can only watch
  devices        list paired devices
  revoke ID      unpair a device (its connections close within a second)
  permission ID operate|view
                 change what a device may do (its connections close within
                 a second and reconnect with it)
  history        recent connections and failed handshakes, newest first
                 (--limit N, default 20)";

/// The windows' control sockets as a gateway source.
#[derive(Default)]
pub struct ControlSource {
    /// Which window's socket owns each pane (from the last topology).
    owner: Mutex<HashMap<u64, PathBuf>>,
    /// `remote-status` notices: (one is being sent, another is due after it).
    notifying: std::sync::Arc<Mutex<(bool, bool)>>,
}

/// How long one window may take to answer `remote-status`.
const NOTICE_TIMEOUT: Duration = Duration::from_millis(500);

impl ControlSource {
    fn owner_of(&self, pane: u64) -> Option<PathBuf> {
        self.owner.lock().unwrap().get(&pane).cloned()
    }

    /// `req` to the window owning `pane`: the cached owner first, after a
    /// miss (pane moved, window gone) look again once. `Ok(None)`: no
    /// window has the pane.
    fn ask(&self, pane: u64, req: &Request) -> std::io::Result<Option<Response>> {
        for attempt in 0..2 {
            let path = match self.owner_of(pane) {
                Some(p) if attempt == 0 => p,
                _ => {
                    self.topology();
                    match self.owner_of(pane) {
                        Some(p) => p,
                        None => return Ok(None),
                    }
                }
            };
            match control::request(&path, req) {
                Ok(r) if !r.ok && r.error.as_deref() == Some("no such pane") => continue,
                Ok(r) => return Ok(Some(r)),
                // the window is gone
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
}

impl Source for ControlSource {
    fn topology(&self) -> Vec<Window> {
        let mut owner = HashMap::new();
        let mut windows = Vec::new();
        for path in control::sockets() {
            let Ok(r) = control::request(&path, &Request::Status) else {
                continue;
            };
            let Some(pid) = r.pid.filter(|_| r.ok) else {
                continue;
            };
            let tabs = r
                .tabs
                .into_iter()
                .map(|t| Tab {
                    index: t.index,
                    title: t.title,
                    active: t.active,
                    panes: t
                        .panes
                        .into_iter()
                        .map(|p| {
                            owner.insert(p.id, path.clone());
                            Pane {
                                id: p.id,
                                active: p.active,
                                cwd: p.cwd,
                                job: p.job,
                                last_command: p.last_command,
                                last_exit: p.last_exit,
                            }
                        })
                        .collect(),
                })
                .collect();
            windows.push(Window { pid, tabs });
        }
        *self.owner.lock().unwrap() = owner;
        windows
    }

    fn blocks(&self, pane: u64, before: Option<u64>, limit: usize) -> Result<Page, SourceError> {
        self.read(Request::Blocks {
            pane,
            before,
            limit,
            meta: false,
        })
    }

    fn blocks_meta(&self, pane: u64, limit: usize) -> Result<Page, SourceError> {
        self.read(meta_request(pane, limit))
    }

    /// Tell every window to re-read `status.json` (its chip), on a thread of
    /// its own: best effort, never blocking the gateway. Changes while a
    /// round is out coalesce into one more round.
    fn status_changed(&self) {
        let state = self.notifying.clone();
        {
            let mut s = state.lock().unwrap();
            if s.0 {
                s.1 = true;
                return;
            }
            s.0 = true;
        }
        let spawned = std::thread::Builder::new()
            .name("remote-notice".into())
            .spawn({
                let state = state.clone();
                move || {
                    loop {
                        for path in control::sockets() {
                            let _ = control::request_within(
                                &path,
                                &Request::RemoteStatus,
                                NOTICE_TIMEOUT,
                            );
                        }
                        let mut s = state.lock().unwrap();
                        if std::mem::take(&mut s.1) {
                            continue;
                        }
                        s.0 = false;
                        break;
                    }
                }
            });
        if spawned.is_err() {
            state.lock().unwrap().0 = false;
        }
    }

    fn input(&self, pane: u64, device: &str, input: Input) -> Result<(), InputError> {
        let (command, keys) = match input {
            Input::Command(c) => (Some(c), None),
            Input::Keys(k) => (None, Some(k.into_iter().map(|k| k.name()).collect())),
        };
        let req = Request::RemoteInput {
            pane,
            device: device.to_string(),
            command,
            keys,
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(()),
            Ok(Some(r)) => Err(input_error(r.error.as_deref().unwrap_or_default())),
            Ok(None) => Err(InputError::NoPane),
            Err(e) => Err(InputError::Unavailable(e.to_string())),
        }
    }
}

impl ControlSource {
    /// A `blocks` request to the window owning its pane.
    fn read(&self, req: Request) -> Result<Page, SourceError> {
        let Request::Blocks { pane, .. } = req else {
            return Err(SourceError::Unavailable("not a blocks request".into()));
        };
        match self.ask(pane, &req) {
            Ok(Some(r)) if r.ok => Ok(Page {
                blocks: r.blocks,
                has_older: r.has_older.unwrap_or(false),
                oldest: r.oldest,
            }),
            Ok(Some(r)) => Err(SourceError::Unavailable(r.error.unwrap_or_default())),
            Ok(None) => Err(SourceError::NoPane),
            Err(e) => Err(SourceError::Unavailable(e.to_string())),
        }
    }
}

/// The `done` probe's read: the newest `limit` blocks without output.
fn meta_request(pane: u64, limit: usize) -> Request {
    Request::Blocks {
        pane,
        before: None,
        limit,
        meta: true,
    }
}

/// A window's `remote-input` error as the gateway's.
fn input_error(msg: &str) -> InputError {
    match msg {
        "busy" => InputError::Busy,
        "no such pane" => InputError::NoPane,
        "remote access is off" => InputError::Denied,
        // control_api changed under the gateway
        m if m.starts_with("not allowed") => InputError::Denied,
        m => InputError::Unavailable(m.to_string()),
    }
}

/// The keys `remote` reads from config.toml.
#[derive(Debug, Default, PartialEq)]
struct RemoteConfig {
    access: bool,
    listen: Option<String>,
    /// `remote_relay` when set.
    relay: Option<String>,
}

/// `remote_access` and `remote_listen` from config.toml, read with the
/// app's own loader each time so a running gateway sees every save. A file
/// the app cannot load (no file, unreadable, invalid) means access off, as
/// the windows' fallback has it.
fn remote_config() -> RemoteConfig {
    remote_config_from(rift_core::Config::try_load())
}

fn remote_config_from(loaded: Result<Option<rift_core::Config>, String>) -> RemoteConfig {
    match loaded {
        Ok(Some(c)) => RemoteConfig {
            access: c.remote_access,
            listen: Some(c.remote_listen),
            relay: Some(c.remote_relay.trim().to_string()).filter(|r| !r.is_empty()),
        },
        _ => RemoteConfig::default(),
    }
}

/// This user's registry directory (`<data dir>/remote`).
pub(crate) fn registry_dir() -> Option<PathBuf> {
    rift_core::shell_integration::data_dir().map(|d| registry::remote_dir(&d))
}

/// `rift-terminal remote …`. Returns the process exit code.
pub fn cli(args: &[String]) -> i32 {
    let Some(dir) = registry_dir() else {
        eprintln!("rift-terminal remote: no data directory (HOME is not set)");
        return 1;
    };
    match args.first().map(String::as_str) {
        Some("pair") => return pair(&dir, &args[1..]),
        Some("devices") if args.len() == 1 => return devices(&dir),
        Some("revoke") if args.len() == 2 => return revoke(&dir, &args[1]),
        Some("permission") if args.len() == 3 => return permission(&dir, &args[1], &args[2]),
        Some("history") => return history(&dir, &args[1..]),
        Some("help" | "--help" | "-h") => {
            println!("{HELP}");
            return 0;
        }
        _ => {}
    }
    let mut listen: Option<String> = None;
    let mut managed = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--listen" => match it.next() {
                Some(v) => listen = Some(v.clone()),
                None => {
                    eprintln!("{HELP}");
                    return 2;
                }
            },
            "--managed" => managed = true,
            other => {
                eprintln!("rift-terminal remote: unknown argument {other:?}\n{HELP}");
                return 2;
            }
        }
    }
    let config = remote_config();
    if managed && !config.access {
        return 0;
    }
    let listen = listen
        .or(config.listen)
        .unwrap_or_else(|| DEFAULT_LISTEN.into());
    let addr: SocketAddr = match listen.parse() {
        Ok(a) => a,
        Err(_) => {
            eprintln!("rift-terminal remote: listen address {listen:?}: expected IP:PORT");
            return 2;
        }
    };
    // one gateway per user: `gateway.lock` decides; status.json only names it
    let already = |dir: &Path| -> i32 {
        if managed {
            return 0;
        }
        match registry::read_status(dir) {
            Some(st) => eprintln!(
                "rift-terminal remote: a gateway is already running (pid {})",
                st.pid
            ),
            None => eprintln!("rift-terminal remote: a gateway is already running"),
        }
        1
    };
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e)
            if e.kind() == std::io::ErrorKind::AddrInUse
                && registry::read_status(&dir).is_some() =>
        {
            return already(&dir);
        }
        Err(e) => {
            eprintln!("rift-terminal remote: {addr}: {e}");
            return 1;
        }
    };
    let mut gw_config = rift_remote::Config::new(
        dir.clone(),
        hostname(),
        os().into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    gw_config.relay = config.relay.clone();
    let gateway = match rift_remote::serve(listener, ControlSource::default(), gw_config) {
        Ok(g) => g,
        // another gateway holds the lock
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => return already(&dir),
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            return 1;
        }
    };
    let bound = gateway.local_addr();
    log::info!("remote: gateway on {bound} (pid {})", std::process::id());
    if !managed {
        println!("Rift remote gateway on {bound} (protocol v2, Noise-encrypted)");
        for a in registry::addresses(bound) {
            println!("  reachable at {a}");
        }
        if let Some(r) = &config.relay {
            println!("  through the relay {r}");
        }
        println!("Pair a phone: rift-terminal remote pair");
        if control::sockets().is_empty() {
            eprintln!("note: no Rift window is listening yet (control_api = \"off\"?)");
        }
        // follow `remote_relay` until killed
        loop {
            std::thread::sleep(MANAGED_CHECK);
            gateway.set_relay(remote_config().relay);
        }
    }
    // managed: follow the config and the windows
    // when no window was first seen (the clock starts at launch too, so a
    // gateway started with no window also ends)
    let mut windowless: Option<Instant> = Some(Instant::now());
    let mut checked_windows: Option<Instant> = None;
    loop {
        std::thread::sleep(MANAGED_CHECK);
        let now = remote_config();
        if !now.access {
            log::info!("remote: remote_access is off; stopping");
            break;
        }
        gateway.set_relay(now.relay);
        if checked_windows.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
            checked_windows = Some(Instant::now());
            if any_window() {
                windowless = None;
            } else if windowless.get_or_insert_with(Instant::now).elapsed() >= NO_WINDOWS_EXIT {
                log::info!("remote: no Rift window for {NO_WINDOWS_EXIT:?}; stopping");
                break;
            }
        }
    }
    drop(gateway);
    0
}

/// A window's control socket answers.
fn any_window() -> bool {
    control::sockets()
        .iter()
        .any(|p| std::os::unix::net::UnixStream::connect(p).is_ok())
}

fn pair(dir: &Path, args: &[String]) -> i32 {
    let permission = match args {
        [] => Permission::Operate,
        [v] if v == "--view" => Permission::View,
        _ => {
            eprintln!("{HELP}");
            return 2;
        }
    };
    let running = registry::read_status(dir);
    // the running gateway's relay, else the configured one
    let relay = running
        .as_ref()
        .and_then(|st| st.relay.as_ref().map(|r| r.url.clone()))
        .or_else(|| remote_config().relay);
    let addresses = match &running {
        Some(st) => st.addresses.clone(),
        None => {
            let listen = remote_config()
                .listen
                .unwrap_or_else(|| DEFAULT_LISTEN.into());
            match listen.parse() {
                Ok(a) => registry::addresses(a),
                Err(_) => {
                    eprintln!("rift-terminal remote: remote_listen {listen:?}: expected IP:PORT");
                    return 1;
                }
            }
        }
    };
    if addresses.is_empty() && relay.is_none() {
        eprintln!("rift-terminal remote: no network address to pair over (not on a network?)");
        return 1;
    }
    let link = match registry::start_pairing_via(
        dir,
        &addresses,
        relay.as_deref(),
        &hostname(),
        permission,
    ) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            return 1;
        }
    };
    print!("{}", registry::qr_terminal(&link.url));
    println!("{}", link.url);
    let left = link
        .expires_at
        .duration_since(SystemTime::now())
        .unwrap_or_default()
        .as_secs();
    println!(
        "Scan with the Rift app. One use, {} access, valid {left} s (until {}).",
        permission.as_str(),
        registry::format_rfc3339(link.expires_at)
    );
    if running.is_none() {
        eprintln!(
            "note: no gateway is running; set remote_access = true in config.toml \
             (or run `rift-terminal remote`) before scanning"
        );
    }
    0
}

fn devices(dir: &Path) -> i32 {
    let all = match registry::devices(dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            return 1;
        }
    };
    if all.is_empty() {
        println!("no paired devices (pair one with `rift-terminal remote pair`)");
        return 0;
    }
    let connected: Vec<String> = registry::read_status(dir)
        .map(|s| s.clients.into_iter().map(|c| c.device).collect())
        .unwrap_or_default();
    println!(
        "{:<10} {:<24} {:<9} {:<8} {:<21} {:<21}",
        "ID", "NAME", "PLATFORM", "ACCESS", "PAIRED", "LAST SEEN"
    );
    for d in all {
        let live = if connected.contains(&d.id) {
            "  connected"
        } else {
            ""
        };
        println!(
            "{:<10} {:<24} {:<9} {:<8} {:<21} {:<21}{live}",
            d.id,
            d.name,
            d.platform,
            d.permission.as_str(),
            registry::format_rfc3339(d.paired_at),
            registry::format_rfc3339(d.last_seen)
        );
    }
    0
}

fn revoke(dir: &Path, id: &str) -> i32 {
    match registry::revoke(dir, id) {
        Ok(true) => {
            println!("revoked {id}");
            0
        }
        Ok(false) => {
            eprintln!("rift-terminal remote: no paired device {id:?} (see `remote devices`)");
            1
        }
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            1
        }
    }
}

fn permission(dir: &Path, id: &str, value: &str) -> i32 {
    let Some(p) = Permission::parse(value) else {
        eprintln!("rift-terminal remote: permission is operate or view, not {value:?}");
        return 2;
    };
    match registry::set_permission(dir, id, p) {
        Ok(true) => {
            println!("{id} may now {}", p.as_str());
            0
        }
        Ok(false) => {
            eprintln!("rift-terminal remote: no paired device {id:?} (see `remote devices`)");
            1
        }
        Err(e) => {
            eprintln!("rift-terminal remote: {e}");
            1
        }
    }
}

/// How long a history entry lasted: `45s`, `12m 3s`, `2h 5m`.
pub(crate) fn duration_text(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

fn history(dir: &Path, args: &[String]) -> i32 {
    let limit = match args {
        [] => 20,
        [flag, n] if flag == "--limit" => match n.parse::<usize>() {
            Ok(n) if n > 0 => n,
            _ => {
                eprintln!("rift-terminal remote: --limit takes a positive number");
                return 2;
            }
        },
        _ => {
            eprintln!("{HELP}");
            return 2;
        }
    };
    let all = registry::history(dir);
    if all.is_empty() {
        println!("no connections yet");
        return 0;
    }
    println!(
        "{:<21} {:<9} {:<24} {:<6} {:<26} {:<9} REASON",
        "START", "DEVICE", "NAME", "VIA", "ADDRESS", "DURATION"
    );
    for e in all.iter().rev().take(limit) {
        println!(
            "{:<21} {:<9} {:<24} {:<6} {:<26} {:<9} {}",
            registry::format_rfc3339(e.start),
            e.device.as_deref().unwrap_or("-"),
            e.name.as_deref().unwrap_or("(unknown)"),
            e.via.as_str(),
            e.address,
            duration_text(e.end.duration_since(e.start).unwrap_or_default()),
            e.reason
        );
    }
    0
}

fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// The computer's name as its owner sees it: macOS's ComputerName ("Sam's MacBook
/// Pro"), Linux's `PRETTY_HOSTNAME` in /etc/machine-info, else the network hostname
/// (which screen readers spell out: "Sams dash MacBook dash Pro dot local").
pub(crate) fn hostname() -> String {
    friendly_name()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(network_hostname)
}

#[cfg(target_os = "macos")]
fn friendly_name() -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn friendly_name() -> Option<String> {
    pretty_hostname(&std::fs::read_to_string("/etc/machine-info").ok()?)
}

/// `PRETTY_HOSTNAME=…` from machine-info(5) text (shell-style, optionally quoted).
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn pretty_hostname(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v = l.trim().strip_prefix("PRETTY_HOSTNAME=")?.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(v);
        Some(v.to_string())
    })
}

fn network_hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer outlives the call and its length is passed
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return "unknown".into();
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

// ── the window side ──

/// Where a phone's input to a pane goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The app's command editor owns the line (the last block is at its
    /// prompt, no full-screen program): a command is accepted and keys are
    /// editor keys.
    Editor,
    /// A program has the pane, or the shell has not started the command
    /// submitted last: a command is `busy`; keys go to the program.
    Raw,
}

/// [`Route`] of a pane: `alt_screen` (a full-screen program), the state of
/// its last block (`None`: no shell integration) and whether a submitted
/// command is still waiting for the shell to start it.
pub fn route(
    alt_screen: bool,
    last: Option<&rift_core::BlockState>,
    submit_pending: bool,
) -> Route {
    match last {
        Some(rift_core::BlockState::Prompt) if !alt_screen && !submit_pending => Route::Editor,
        _ => Route::Raw,
    }
}

/// Longest command shown in the desktop's toast, in characters.
const TOAST_COMMAND: usize = 60;
/// Longest device name shown in the toast, in characters.
const TOAST_DEVICE: usize = 40;

/// `text` without control characters, cut to `max` characters with `…`.
fn shown(text: &str, max: usize) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.chars().count() <= max {
        return clean.to_string();
    }
    let mut out: String = clean.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// The desktop's toast for a phone's command: `<device> ran <command>`.
pub fn ran_toast(device: &str, command: &str) -> String {
    format!(
        "{} ran {}",
        shown(device, TOAST_DEVICE),
        shown(command, TOAST_COMMAND)
    )
}

/// The desktop's toast for a phone's keys: `<device> sent ctrl+c enter`.
pub fn sent_toast(device: &str, keys: &[rift_remote::Key]) -> String {
    let names: Vec<String> = keys.iter().map(|k| k.name()).collect();
    format!(
        "{} sent {}",
        shown(device, TOAST_DEVICE),
        shown(&names.join(" "), TOAST_COMMAND)
    )
}

/// Start a managed gateway for this user (`<this exe> remote --managed`,
/// the same environment): detached in its own process group, its output
/// in `<dir>/gateway.log` (started over past 1 MiB), reaped by a thread
/// when it exits. Returns its pid. A gateway already holding the lock
/// makes the new one exit at once.
pub fn spawn_managed(dir: &Path) -> std::io::Result<u32> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::os::unix::process::CommandExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let path = dir.join("gateway.log");
    let big = std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20);
    let mut open = std::fs::OpenOptions::new();
    open.create(true).mode(0o600);
    if big {
        open.write(true).truncate(true);
    } else {
        open.append(true);
    }
    let log = open.open(&path)?;
    let err = log.try_clone()?;
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .args(["remote", "--managed"])
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(err)
        .process_group(0)
        .spawn()?;
    let pid = child.id();
    // no zombie when it exits while this window lives on
    std::thread::Builder::new()
        .name("remote-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_hostname_comes_from_machine_info() {
        let t = "ICON_NAME=computer-laptop\nPRETTY_HOSTNAME=\"Sam's Laptop\"\nCHASSIS=laptop\n";
        assert_eq!(pretty_hostname(t).as_deref(), Some("Sam's Laptop"));
        assert_eq!(
            pretty_hostname("PRETTY_HOSTNAME=build-box").as_deref(),
            Some("build-box")
        );
        assert_eq!(pretty_hostname("CHASSIS=desktop\n"), None);
        // whatever the source, the name is never empty
        assert!(!hostname().trim().is_empty());
    }

    #[test]
    fn commands_need_the_editor_at_a_prompt() {
        use rift_core::BlockState as B;
        let running = B::Running {
            started: Instant::now(),
        };
        let done = B::Done {
            exit: Some(0),
            duration: Duration::ZERO,
        };
        assert_eq!(route(false, Some(&B::Prompt), false), Route::Editor);
        // a full-screen program, a running command, a command the shell has
        // not started yet, no shell integration: busy for commands
        assert_eq!(route(true, Some(&B::Prompt), false), Route::Raw);
        assert_eq!(route(false, Some(&running), false), Route::Raw);
        assert_eq!(route(false, Some(&B::Prompt), true), Route::Raw);
        assert_eq!(route(false, Some(&done), false), Route::Raw);
        assert_eq!(route(false, None, false), Route::Raw);
    }

    #[test]
    fn toasts_name_the_device_and_stay_short() {
        assert_eq!(
            ran_toast("Sam's iPhone", "cargo test"),
            "Sam's iPhone ran cargo test"
        );
        let long = "x".repeat(200);
        let t = ran_toast("p", &long);
        assert_eq!(t.chars().count(), "p ran ".len() + TOAST_COMMAND);
        assert!(t.ends_with('…'));
        assert_eq!(ran_toast("a\u{1b}[31mb\n", "ls"), "a[31mb ran ls");
        let keys = [rift_remote::Key::Ctrl('c'), rift_remote::Key::Enter];
        assert_eq!(sent_toast("phone", &keys), "phone sent ctrl+c enter");
    }

    #[test]
    fn window_errors_map_to_input_errors() {
        assert_eq!(input_error("busy"), InputError::Busy);
        assert_eq!(input_error("no such pane"), InputError::NoPane);
        assert_eq!(input_error("remote access is off"), InputError::Denied);
        assert_eq!(
            input_error("not allowed: control permissions changed"),
            InputError::Denied
        );
        assert_eq!(
            input_error("remote input is not available"),
            InputError::Unavailable("remote input is not available".into())
        );
        assert!(!hostname().is_empty());
    }

    fn from_text(text: &str) -> RemoteConfig {
        let dir = std::env::temp_dir().join(format!(
            "rift-remote-cfg-{}-{}",
            std::process::id(),
            text.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        let got = remote_config_from(rift_core::Config::try_load_from(&path));
        std::fs::remove_dir_all(&dir).ok();
        got
    }

    #[test]
    fn remote_keys_are_read_with_the_apps_loader() {
        // no file: defaults (off)
        assert_eq!(remote_config_from(Ok(None)), RemoteConfig::default());
        assert_eq!(
            from_text(""),
            RemoteConfig {
                access: false,
                listen: Some("0.0.0.0:7878".into()),
                relay: None,
            }
        );
        assert_eq!(
            from_text(
                "remote_access = true\nremote_listen = \"127.0.0.1:9\"\n\
                 remote_relay = \" wss://relay.example.com \"\n"
            ),
            RemoteConfig {
                access: true,
                listen: Some("127.0.0.1:9".into()),
                relay: Some("wss://relay.example.com".into()),
            }
        );
        assert_eq!(from_text("remote_relay = \"  \"\n").relay, None);
        // only a real boolean turns it on, and a file the app cannot load
        // keeps it off (the windows fall back the same way)
        for text in [
            "remote_access = \"true\"",
            "remote_access = 1",
            "[x]\nremote_access = true",
            "remote_access = true\nbroken = [",
            "remote_access = true\nfont_size = \"big\"",
        ] {
            assert!(!from_text(text).access, "{text}");
        }
    }

    #[test]
    fn durations_read_short() {
        assert_eq!(duration_text(Duration::from_secs(0)), "0s");
        assert_eq!(duration_text(Duration::from_secs(59)), "59s");
        assert_eq!(duration_text(Duration::from_secs(60)), "1m 0s");
        assert_eq!(duration_text(Duration::from_secs(3599)), "59m 59s");
        assert_eq!(duration_text(Duration::from_secs(7500)), "2h 5m");
    }

    #[test]
    fn the_done_probe_asks_for_metadata_only() {
        let line = serde_json::to_string(&meta_request(12, 4)).unwrap();
        assert_eq!(
            line,
            r#"{"cmd":"blocks","pane":12,"before":null,"limit":4,"meta":true}"#
        );
    }
}
