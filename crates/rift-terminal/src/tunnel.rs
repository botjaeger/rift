//! `remote_tunnel`: a relay on this computer behind a Cloudflare quick
//! tunnel, so phones reach the gateway from any network without a server
//! or a router change. The gateway (`remote.rs`) runs it: the relay
//! (`rift_remote::relay`) listens on a loopback port, `cloudflared tunnel
//! --url` gives that port a public `https://….trycloudflare.com` address,
//! and the gateway registers at its `wss://` form. The relay and Cloudflare
//! only ever see Noise ciphertext. The address is new on every start, so
//! phones pair again after one.
//!
//! The state is written to `<remote dir>/tunnel.json` for Settings.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where Homebrew lives (Apple silicon, Intel, Linux).
pub const BREW: [&str; 3] = [
    "/opt/homebrew/bin/brew",
    "/usr/local/bin/brew",
    "/home/linuxbrew/.linuxbrew/bin/brew",
];

/// Homebrew is installed (Settings offers `brew install cloudflared`).
pub fn has_brew() -> bool {
    BREW.iter().any(|p| Path::new(p).is_file())
}

/// Cloudflare's cloudflared downloads, for systems without Homebrew.
pub const DOWNLOAD_URL: &str =
    "https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/";

/// The tunnel's address is used this long after cloudflared prints it.
const DNS_GRACE: Duration = Duration::from_secs(20);

/// A tunnel whose cloudflared exited is started again after this long.
const RETRY: Duration = Duration::from_secs(30);
/// Without cloudflared, look for it again this often (an install takes
/// effect without a restart).
const LOOK_AGAIN: Duration = Duration::from_secs(5);

/// What Settings shows about the tunnel (`tunnel.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum TunnelState {
    /// cloudflared is not installed.
    Missing,
    /// cloudflared runs; no public address yet.
    Starting,
    /// The relay's public address (`wss://…`).
    Ready { url: String },
    /// cloudflared exited: its last line (it is started again).
    Failed { detail: String },
}

/// `cloudflared` on PATH, else where Homebrew and the packages put it (an
/// app started from the Finder gets a short PATH). `RIFT_CLOUDFLARED`
/// names the binary instead (empty: none, for tests).
pub fn find_cloudflared() -> Option<PathBuf> {
    if let Some(bin) = std::env::var_os("RIFT_CLOUDFLARED") {
        return Some(PathBuf::from(bin)).filter(|p| p.is_file());
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(
            ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
                .into_iter()
                .map(PathBuf::from),
        )
        .map(|d| d.join("cloudflared"))
        .find(|p| p.is_file())
}

/// The quick tunnel's address in a line cloudflared logs.
fn tunnel_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || ":/.-".contains(c)))
        .unwrap_or(rest.len());
    let url = &rest[..end];
    let host = url.strip_prefix("https://")?;
    (host.ends_with(".trycloudflare.com") && host != "api.trycloudflare.com")
        .then(|| format!("wss://{host}"))
}

/// The running relay and cloudflared.
struct Running {
    /// Dropped last: stops the relay.
    relay: Option<rift_remote::relay::Relay>,
    child: Child,
    /// Held open: cloudflared's wrapper ends it once this closes, so it
    /// never outlives the gateway (even one that is killed).
    stdin: Option<ChildStdin>,
    url: Arc<Mutex<Option<(String, Instant)>>>,
    last_line: Arc<Mutex<String>>,
}

impl Running {
    /// `gateway_id`: the routing id of this computer's gateway, the only one
    /// the relay lets register (the public address is no open relay).
    fn start(cloudflared: &Path, gateway_id: String) -> std::io::Result<Running> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        // Cloudflare appends the phone's address to X-Forwarded-For, so the
        // rate limits count real clients; only this computer reaches the port
        let config = rift_remote::relay::RelayConfig {
            trust_forwarded: true,
            only_ids: Some(vec![gateway_id]),
            max_sockets: rift_remote::relay::ONE_GATEWAY_SOCKETS,
            ..Default::default()
        };
        let relay = rift_remote::relay::serve_relay(listener, config)?;
        let spawned = Command::new("/bin/sh")
            .args([
                "-c",
                // background jobs read /dev/null: the watcher reads our
                // pipe through fd 3 and ends cloudflared at its EOF
                "exec 3<&0; \"$0\" \"$@\" </dev/null & c=$!; \
                 (read _ <&3; kill $c 2>/dev/null) & wait $c",
            ])
            .arg(cloudflared)
            .args(["tunnel", "--no-autoupdate", "--url"])
            .arg(format!("http://127.0.0.1:{port}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        // a failed spawn drops (stops) the relay
        let mut child = spawned?;
        let url = Arc::new(Mutex::new(None));
        let last_line = Arc::new(Mutex::new(String::new()));
        if let Some(err) = child.stderr.take() {
            let (url, last_line) = (url.clone(), last_line.clone());
            std::thread::Builder::new()
                .name("remote-tunnel-log".into())
                .spawn(move || {
                    for line in BufReader::new(err).lines().map_while(Result::ok) {
                        if let Some(u) = tunnel_url(&line) {
                            log::info!("remote: tunnel at {u}");
                            *url.lock().unwrap() = Some((u, Instant::now()));
                        }
                        log::debug!("cloudflared: {line}");
                        *last_line.lock().unwrap() = line;
                    }
                })?;
        }
        let stdin = child.stdin.take();
        Ok(Running {
            relay: Some(relay),
            child,
            stdin,
            url,
            last_line,
        })
    }

    /// cloudflared has exited.
    fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // closing stdin makes the wrapper kill cloudflared
        drop(self.stdin.take());
        let until = Instant::now() + Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        drop(self.relay.take());
    }
}

/// The tunnel as the gateway drives it: [`Tunnel::poll`] once per check.
pub struct Tunnel {
    dir: PathBuf,
    running: Option<Running>,
    state: Option<TunnelState>,
    /// When to (re)try starting.
    next_try: Instant,
}

impl Tunnel {
    /// `dir`: the registry directory `tunnel.json` goes to (a leftover of a
    /// gateway that did not end cleanly is removed).
    pub fn new(dir: &Path) -> Tunnel {
        let _ = std::fs::remove_file(dir.join("tunnel.json"));
        Tunnel {
            dir: dir.to_path_buf(),
            running: None,
            state: None,
            next_try: Instant::now(),
        }
    }

    /// Follow `on`; returns the relay URL to register at while it is ready.
    pub fn poll(&mut self, on: bool) -> Option<String> {
        if !on {
            if self.running.take().is_some() {
                log::info!("remote: tunnel stopped");
            }
            if self.state.take().is_some() {
                let _ = std::fs::remove_file(self.dir.join("tunnel.json"));
            }
            self.next_try = Instant::now();
            return None;
        }
        if let Some(r) = &mut self.running
            && r.exited()
        {
            let detail = std::mem::take(&mut *r.last_line.lock().unwrap());
            let detail = if detail.is_empty() {
                "cloudflared exited".to_string()
            } else {
                detail
            };
            log::warn!("remote: tunnel ended: {detail}");
            self.running = None;
            self.next_try = Instant::now() + RETRY;
            self.set(TunnelState::Failed { detail });
        }
        if self.running.is_none() && Instant::now() >= self.next_try {
            match find_cloudflared() {
                None => {
                    self.next_try = Instant::now() + LOOK_AGAIN;
                    self.set(TunnelState::Missing);
                }
                Some(bin) => match rift_remote::registry::gateway_public_key(&self.dir)
                    .and_then(|key| Running::start(&bin, rift_remote::registry::routing_id(&key)))
                {
                    Ok(r) => {
                        self.running = Some(r);
                        self.set(TunnelState::Starting);
                    }
                    Err(e) => {
                        self.next_try = Instant::now() + RETRY;
                        self.set(TunnelState::Failed {
                            detail: format!("cannot start the tunnel: {e}"),
                        });
                    }
                },
            }
        }
        let (url, seen) = self.running.as_ref()?.url.lock().unwrap().clone()?;
        // a lookup before the new name is published is cached as missing
        // and holds registration back for minutes: wait it out first
        if seen.elapsed() < DNS_GRACE {
            return None;
        }
        self.set(TunnelState::Ready { url: url.clone() });
        Some(url)
    }

    fn set(&mut self, state: TunnelState) {
        if self.state.as_ref() == Some(&state) {
            return;
        }
        if let Ok(text) = serde_json::to_string(&state) {
            let path = self.dir.join("tunnel.json");
            let tmp = self.dir.join("tunnel.json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        self.state = Some(state);
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.poll(false);
    }
}

/// Settings' read of `tunnel.json` (None: no tunnel running or wanted).
pub fn read_state(dir: &Path) -> Option<TunnelState> {
    let text = std::fs::read_to_string(dir.join("tunnel.json")).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_quick_tunnel_address_in_cloudflared_output() {
        let line = "2026-10-14T01:30:41Z INF |  https://prisoners-sin-wool-deposit.trycloudflare.com                                     |";
        assert_eq!(
            tunnel_url(line).as_deref(),
            Some("wss://prisoners-sin-wool-deposit.trycloudflare.com")
        );
        // the API it asks for the tunnel, and other links, are not it
        assert_eq!(
            tunnel_url("INF Requesting new quick Tunnel on https://api.trycloudflare.com..."),
            None
        );
        assert_eq!(
            tunnel_url("see https://developers.cloudflare.com/x for details"),
            None
        );
        assert_eq!(tunnel_url("no address here"), None);
    }

    #[test]
    fn state_round_trips_through_json() {
        for s in [
            TunnelState::Missing,
            TunnelState::Starting,
            TunnelState::Ready {
                url: "wss://a.trycloudflare.com".into(),
            },
            TunnelState::Failed {
                detail: "boom".into(),
            },
        ] {
            let text = serde_json::to_string(&s).unwrap();
            assert_eq!(serde_json::from_str::<TunnelState>(&text).unwrap(), s);
        }
        let dir = std::env::temp_dir().join(format!("rift-tunnel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut t = Tunnel::new(&dir);
        t.set(TunnelState::Missing);
        assert_eq!(read_state(&dir), Some(TunnelState::Missing));
        // off removes the file
        assert_eq!(t.poll(false), None);
        assert_eq!(read_state(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fake cloudflared: logs a tunnel address, writes its pid, then
    /// stays up until it is killed.
    fn fake_cloudflared(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("cloudflared");
        let script = format!(
            "#!/bin/sh\necho $$ > '{}'\n\
             echo 'INF |  https://fake-quick-tunnel.trycloudflare.com  |' >&2\n\
             exec sleep 600\n",
            dir.join("pid").display()
        );
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks that the process exists
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn the_tunnel_finds_its_address_and_never_outlives_its_owner() {
        let dir = std::env::temp_dir().join(format!("rift-tunnel-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = fake_cloudflared(&dir);
        let running = Running::start(&bin, "id".into()).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while running.url.lock().unwrap().is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            running.url.lock().unwrap().as_ref().map(|u| u.0.clone()),
            Some("wss://fake-quick-tunnel.trycloudflare.com".into())
        );
        let pid: i32 = std::fs::read_to_string(dir.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(pid));
        drop(running);
        let until = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(pid), "cloudflared outlived the tunnel");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
