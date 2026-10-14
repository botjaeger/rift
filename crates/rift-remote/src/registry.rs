//! The gateway's files under `<data dir>/remote/` (`docs/remote-protocol.md`
//! § Desktop side): the static key, paired devices, live pairing secrets,
//! the running gateway's status and its lock.
//!
//! The directory is 0700 and every file 0600. Files are replaced atomically
//! (written to a temporary file in the same directory, synced, renamed), and
//! read-modify-write changes hold an exclusive `flock` on the directory, so
//! the gateway, the CLI and the desktop UI can change them concurrently.
//!
//! This module is what the desktop UI calls: [`remote_dir`],
//! [`gateway_public_key`], [`start_pairing`], [`addresses`], [`devices`],
//! [`revoke`], [`read_status`] / [`gateway_running`] / [`status_modified`],
//! [`qr_matrix`] / [`qr_terminal`].

pub use crate::protocol::Permission;
use crate::transport::{self, KEY_LEN};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a pairing secret is valid.
pub const PAIRING_TTL: Duration = Duration::from_secs(120);
/// Random bytes in a pairing secret.
pub const SECRET_LEN: usize = 16;
/// Longest device name kept (characters); longer names are cut.
pub const MAX_NAME: usize = 64;

const KEY_FILE: &str = "gateway.key";
const DEVICES_FILE: &str = "devices.json";
const PAIRING_FILE: &str = "pairing.json";
const STATUS_FILE: &str = "status.json";
const LOCK_FILE: &str = "gateway.lock";
const RELAY_FILE: &str = "relay.json";
const HISTORY_FILE: &str = "history.jsonl";

/// Entries `history.jsonl` keeps (the newest).
pub const MAX_HISTORY: usize = 500;
/// Random bytes in the gateway's relay secret.
pub const RELAY_SECRET_LEN: usize = 32;
/// Bytes of SHA-256 over the gateway's static key that name it at a relay.
pub const ROUTING_ID_LEN: usize = 16;

/// `<data_dir>/remote`.
pub fn remote_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("remote")
}

/// A paired device (`devices.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "DeviceFile")]
pub struct Device {
    /// Short random id (hex), shown in `devices` and used by `revoke`.
    pub id: String,
    /// The name the device gave when it paired.
    pub name: String,
    /// `ios`, `android`, … as the device reported it.
    pub platform: String,
    /// The device's static X25519 public key, base64url.
    pub key: String,
    pub permission: Permission,
    #[serde(with = "rfc3339")]
    pub paired_at: SystemTime,
    #[serde(with = "rfc3339")]
    pub last_seen: SystemTime,
    /// Where finish alerts go through Expo while the app is closed (`push`
    /// with a token), if the device asked for them. The same shape as before
    /// ntfy, so an older Rift still reads the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<ExpoReg>,
    /// Where they go through ntfy (`push` with `ntfy`). A field older Rift
    /// builds ignore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ntfy: Option<NtfyReg>,
}

/// [`Device`] as read: `push` may also be the interim shape with a null
/// `token` and an `ntfy` URL, written by development builds before
/// [`NtfyReg`] existed; it is read as the two halves so one such entry never
/// makes the whole file unreadable.
#[derive(Deserialize)]
struct DeviceFile {
    id: String,
    name: String,
    platform: String,
    key: String,
    permission: Permission,
    #[serde(with = "rfc3339")]
    paired_at: SystemTime,
    #[serde(with = "rfc3339")]
    last_seen: SystemTime,
    #[serde(default)]
    push: Option<PushFile>,
    #[serde(default)]
    ntfy: Option<NtfyReg>,
}

#[derive(Deserialize)]
struct PushFile {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    ntfy: Option<String>,
    long_ms: Option<u64>,
    show_command: bool,
    #[serde(default)]
    failures_only: bool,
}

impl From<DeviceFile> for Device {
    fn from(f: DeviceFile) -> Device {
        let (push, interim_ntfy) = match f.push {
            Some(p) => PushReg {
                token: p.token,
                ntfy: p.ntfy,
                long_ms: p.long_ms,
                show_command: p.show_command,
                failures_only: p.failures_only,
            }
            .split(),
            None => (None, None),
        };
        Device {
            id: f.id,
            name: f.name,
            platform: f.platform,
            key: f.key,
            permission: f.permission,
            paired_at: f.paired_at,
            last_seen: f.last_seen,
            push,
            ntfy: f.ntfy.or(interim_ntfy),
        }
    }
}

/// A device's Expo registration as `devices.json` stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpoReg {
    /// The Expo push token (`ExponentPushToken[…]`).
    pub token: String,
    pub long_ms: Option<u64>,
    pub show_command: bool,
    /// Absent in files written before Phase 3.
    #[serde(default)]
    pub failures_only: bool,
}

/// A device's ntfy registration as `devices.json` stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NtfyReg {
    /// The topic URL (`https://<host>/<topic>`).
    pub url: String,
    pub long_ms: Option<u64>,
    pub show_command: bool,
    pub failures_only: bool,
}

/// A device's alert registration, as the last `push` set it
/// (`docs/remote-protocol.md` § Push): stored as [`ExpoReg`] and
/// [`NtfyReg`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReg {
    /// The Expo push token (`ExponentPushToken[…]`), if any.
    pub token: Option<String>,
    /// The ntfy topic URL (`https://<host>/<topic>`), if any.
    pub ntfy: Option<String>,
    /// Alert for every command that ran at least this long; `None`: only
    /// for the panes the device asked about (`notify`).
    pub long_ms: Option<u64>,
    /// Put the command line in the alert (it then passes through the push
    /// services).
    pub show_command: bool,
    /// The `long_ms` alerts only for commands that failed (`notify` alerts
    /// either way).
    pub failures_only: bool,
}

impl PushReg {
    /// The stored halves (`None`, `None` for a registration with neither).
    pub(crate) fn split(self) -> (Option<ExpoReg>, Option<NtfyReg>) {
        let expo = self.token.map(|token| ExpoReg {
            token,
            long_ms: self.long_ms,
            show_command: self.show_command,
            failures_only: self.failures_only,
        });
        let ntfy = self.ntfy.map(|url| NtfyReg {
            url,
            long_ms: self.long_ms,
            show_command: self.show_command,
            failures_only: self.failures_only,
        });
        (expo, ntfy)
    }
}

impl Device {
    /// The device's alert registration, if it has one (Expo, ntfy or both).
    pub fn alerts(&self) -> Option<PushReg> {
        let (long_ms, show_command, failures_only) = match (&self.push, &self.ntfy) {
            (Some(e), _) => (e.long_ms, e.show_command, e.failures_only),
            (None, Some(n)) => (n.long_ms, n.show_command, n.failures_only),
            (None, None) => return None,
        };
        Some(PushReg {
            token: self.push.as_ref().map(|e| e.token.clone()),
            ntfy: self.ntfy.as_ref().map(|n| n.url.clone()),
            long_ms,
            show_command,
            failures_only,
        })
    }

    /// The decoded static key (`None` when the stored text is not one).
    pub fn key_bytes(&self) -> Option<[u8; KEY_LEN]> {
        decode_key(&self.key)
    }
}

/// A live pairing secret (`pairing.json`): never the secret itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Secret {
    /// SHA-256 of the secret's 16 bytes, lowercase hex.
    hash: String,
    permission: Permission,
    #[serde(with = "rfc3339")]
    expires_at: SystemTime,
}

/// How a connection reached the gateway.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// Directly, on the gateway's listening address.
    #[default]
    Lan,
    /// Spliced by the relay (`remote_relay`).
    Relay,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Lan => "lan",
            Via::Relay => "relay",
        }
    }
}

/// A connected client in [`Status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    /// Device id.
    pub device: String,
    /// Device name.
    pub name: String,
    #[serde(with = "rfc3339")]
    pub since: SystemTime,
    #[serde(default)]
    pub via: Via,
}

/// The gateway's relay connection in [`Status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayState {
    /// Registered: phones can connect through it.
    Connected,
    /// Dialing or registering (first try, or after a drop).
    Connecting,
    /// The last try failed (`error` says why); it is tried again.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayStatus {
    pub url: String,
    pub state: RelayState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The running gateway (`status.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    /// The addresses it is reachable on (what pairing links list).
    pub addresses: Vec<SocketAddr>,
    pub clients: Vec<Client>,
    /// The relay connection, when `remote_relay` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<RelayStatus>,
}

/// One closed connection or failed handshake (`history.jsonl`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Device id; `None` for a handshake that did not authenticate.
    pub device: Option<String>,
    /// Device name (as paired); `None` with `device`.
    pub name: Option<String>,
    pub via: Via,
    /// The peer's address (`IP:PORT` on the LAN; the relay's host for
    /// relay connections, which never learn the phone's address).
    pub address: String,
    #[serde(with = "rfc3339")]
    pub start: SystemTime,
    #[serde(with = "rfc3339")]
    pub end: SystemTime,
    /// Why it ended: the gateway's reasons (`closed`, `idle`,
    /// `revoked`, `permission`, `auth`, `handshake`, …).
    pub reason: String,
}

/// A pairing link from [`start_pairing`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingLink {
    /// `rift-remote://pair?a=…&k=…&s=…&n=…&p=…`
    pub url: String,
    pub expires_at: SystemTime,
    pub permission: Permission,
}

fn ensure_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Replace `dir/name` with `body` atomically (0600).
fn write_atomic(dir: &Path, name: &str, body: &[u8]) -> io::Result<()> {
    ensure_dir(dir)?;
    let suffix: [u8; 6] = transport::random_bytes()?;
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        hex(&suffix)
    ));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(body)?;
        f.sync_all()?;
        std::fs::rename(&tmp, dir.join(name))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Run `f` holding the registry's exclusive lock (a `flock` on the
/// directory itself, so no extra file is needed).
fn locked<T>(dir: &Path, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    ensure_dir(dir)?;
    let handle = File::open(dir)?;
    loop {
        // SAFETY: flock on a descriptor we own; released when it closes
        if unsafe { libc::flock(handle.as_raw_fd(), libc::LOCK_EX) } == 0 {
            break;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
    let out = f();
    drop(handle);
    out
}

/// A JSON list file; missing = empty, unreadable or corrupt = error.
fn read_list<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Vec<T>> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {e}", path.display()),
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

fn write_json<T: Serialize>(dir: &Path, name: &str, value: &T) -> io::Result<()> {
    let body = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(dir, name, &body)
}

/// The gateway's static private key, created on first use.
pub(crate) fn gateway_private_key(dir: &Path) -> io::Result<[u8; KEY_LEN]> {
    let path = dir.join(KEY_FILE);
    let read = |path: &Path| -> io::Result<Option<[u8; KEY_LEN]>> {
        match std::fs::read(path) {
            Ok(b) => <[u8; KEY_LEN]>::try_from(b.as_slice())
                .map(Some)
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{}: not a 32-byte key", path.display()),
                    )
                }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    };
    if let Some(k) = read(&path)? {
        return Ok(k);
    }
    locked(dir, || {
        if let Some(k) = read(&path)? {
            return Ok(k);
        }
        let key = transport::generate_private_key().map_err(|e| io::Error::other(e.to_string()))?;
        write_atomic(dir, KEY_FILE, &key)?;
        Ok(key)
    })
}

/// The gateway's static public key (what clients pin), creating the key
/// pair on first use.
pub fn gateway_public_key(dir: &Path) -> io::Result<[u8; KEY_LEN]> {
    gateway_private_key(dir).map(|k| transport::public_key(&k))
}

/// Create a one-time pairing secret (valid [`PAIRING_TTL`]) and the link
/// that carries it: the addresses to try, the gateway's public key, the
/// secret, the host name to show and the permission.
pub fn start_pairing(
    dir: &Path,
    addresses: &[SocketAddr],
    host: &str,
    permission: Permission,
) -> io::Result<PairingLink> {
    start_pairing_via(dir, addresses, None, host, permission)
}

/// [`start_pairing`] for a gateway that also has a relay: the link gains
/// `r=<relay URL>` (percent-encoded) after `p`.
pub fn start_pairing_via(
    dir: &Path,
    addresses: &[SocketAddr],
    relay: Option<&str>,
    host: &str,
    permission: Permission,
) -> io::Result<PairingLink> {
    let key = gateway_public_key(dir)?;
    let secret: [u8; SECRET_LEN] = transport::random_bytes()?;
    // whole seconds: the file stores RFC 3339 seconds
    let now = whole_secs(SystemTime::now());
    let expires_at = now + PAIRING_TTL;
    locked(dir, || {
        let path = dir.join(PAIRING_FILE);
        // a corrupt file only loses secrets nobody can use any more
        let mut live: Vec<Secret> = read_list(&path).unwrap_or_default();
        live.retain(|s| s.expires_at > now);
        live.push(Secret {
            hash: hash_secret(&secret),
            permission,
            expires_at,
        });
        write_json(dir, PAIRING_FILE, &live)
    })?;
    let a: Vec<String> = addresses.iter().map(|a| a.to_string()).collect();
    let mut url = format!(
        "rift-remote://pair?a={}&k={}&s={}&n={}&p={}",
        a.join(","),
        B64.encode(key),
        B64.encode(secret),
        percent_encode(host),
        permission.as_str()
    );
    if let Some(r) = relay.filter(|r| !r.is_empty()) {
        url.push_str("&r=");
        url.push_str(&percent_encode(r));
    }
    Ok(PairingLink {
        url,
        expires_at,
        permission,
    })
}

/// Every paired device (none when the file does not exist yet).
pub fn devices(dir: &Path) -> io::Result<Vec<Device>> {
    read_list(&dir.join(DEVICES_FILE))
}

/// Remove the device `id`; false when there is none. Its open connections
/// close within a second (the gateway watches `devices.json`).
pub fn revoke(dir: &Path, id: &str) -> io::Result<bool> {
    locked(dir, || {
        let mut all = devices(dir)?;
        let before = all.len();
        all.retain(|d| d.id != id);
        if all.len() == before {
            return Ok(false);
        }
        write_json(dir, DEVICES_FILE, &all)?;
        Ok(true)
    })
}

/// Set the device `id`'s permission; false when there is none. Its open
/// connections close within a second (the gateway watches `devices.json`),
/// so its next handshake carries the new permission.
pub fn set_permission(dir: &Path, id: &str, permission: Permission) -> io::Result<bool> {
    locked(dir, || {
        let mut all = devices(dir)?;
        let Some(d) = all.iter_mut().find(|d| d.id == id) else {
            return Ok(false);
        };
        if d.permission != permission {
            d.permission = permission;
            write_json(dir, DEVICES_FILE, &all)?;
        }
        Ok(true)
    })
}

/// Set (or with `None` clear) device `id`'s push registration. `Ok(false)`
/// when no such device is paired.
pub fn set_push(dir: &Path, id: &str, push: Option<PushReg>) -> io::Result<bool> {
    let (expo, ntfy) = push.map(PushReg::split).unwrap_or_default();
    locked(dir, || {
        let mut all = devices(dir)?;
        let Some(d) = all.iter_mut().find(|d| d.id == id) else {
            return Ok(false);
        };
        if d.push != expo || d.ntfy != ntfy {
            d.push = expo;
            d.ntfy = ntfy;
            write_json(dir, DEVICES_FILE, &all)?;
        }
        Ok(true)
    })
}

/// Remove device `id`'s Expo token if it is still `token` (the push service
/// no longer knows it); its ntfy registration stays. `Ok(false)`: nothing
/// removed.
pub fn clear_token(dir: &Path, id: &str, token: &str) -> io::Result<bool> {
    locked(dir, || {
        let mut all = devices(dir)?;
        let Some(d) = all
            .iter_mut()
            .find(|d| d.id == id && d.push.as_ref().is_some_and(|e| e.token == token))
        else {
            return Ok(false);
        };
        d.push = None;
        write_json(dir, DEVICES_FILE, &all)?;
        Ok(true)
    })
}

/// The routing id that names the gateway with static public key `public`
/// at a relay: base64url of the first 16 bytes of its SHA-256.
pub fn routing_id(public: &[u8; KEY_LEN]) -> String {
    B64.encode(&Sha256::digest(public)[..ROUTING_ID_LEN])
}

#[derive(Serialize, Deserialize)]
struct RelayFile {
    /// base64url, [`RELAY_SECRET_LEN`] bytes.
    secret: String,
}

/// The secret the gateway registers its routing id with at a relay
/// (`relay.json`), created once, so restarts keep the id.
pub fn relay_secret(dir: &Path) -> io::Result<[u8; RELAY_SECRET_LEN]> {
    let path = dir.join(RELAY_FILE);
    let read = |path: &Path| -> io::Result<Option<[u8; RELAY_SECRET_LEN]>> {
        match std::fs::read(path) {
            Ok(b) => {
                let bad = || {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{}: not a relay secret", path.display()),
                    )
                };
                let f: RelayFile = serde_json::from_slice(&b).map_err(|_| bad())?;
                let bytes = B64.decode(f.secret.as_bytes()).map_err(|_| bad())?;
                bytes.try_into().map(Some).map_err(|_| bad())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    };
    if let Some(s) = read(&path)? {
        return Ok(s);
    }
    locked(dir, || {
        if let Some(s) = read(&path)? {
            return Ok(s);
        }
        let secret: [u8; RELAY_SECRET_LEN] = transport::random_bytes()?;
        write_json(
            dir,
            RELAY_FILE,
            &RelayFile {
                secret: B64.encode(secret),
            },
        )?;
        Ok(secret)
    })
}

/// Append `entry` to `history.jsonl` (0600), keeping the newest
/// [`MAX_HISTORY`]: a plain append while there is room, an atomic rewrite
/// of the newest entries when it is full.
pub(crate) fn append_history(dir: &Path, entry: &Entry) -> io::Result<()> {
    let line = serde_json::to_string(entry).map_err(io::Error::other)?;
    locked(dir, || {
        let path = dir.join(HISTORY_FILE);
        let existing = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let lines: Vec<&str> = existing.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() < MAX_HISTORY && (existing.is_empty() || existing.ends_with('\n')) {
            let mut f = OpenOptions::new()
                .append(true)
                .create(true)
                .mode(0o600)
                .open(&path)?;
            return f.write_all(format!("{line}\n").as_bytes());
        }
        let mut body: String = trim_history(&lines)
            .iter()
            .map(|l| format!("{l}\n"))
            .collect();
        body.push_str(&line);
        body.push('\n');
        write_atomic(dir, HISTORY_FILE, body.as_bytes())
    })
}

/// The lines a full history keeps to make room for one more: the oldest
/// failed handshakes (no device) and unreadable lines go first, so a flood
/// of refused attempts (anyone can cause them) never pushes out a paired
/// device's connections; only then the oldest entries.
fn trim_history<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut excess = (lines.len() + 1).saturating_sub(MAX_HISTORY);
    let anonymous = |l: &str| serde_json::from_str::<Entry>(l).map_or(true, |e| e.device.is_none());
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    for l in lines {
        if excess > 0 && anonymous(l) {
            excess -= 1;
        } else {
            kept.push(l);
        }
    }
    kept.split_off(excess.min(kept.len()))
}

/// The connection history, oldest first (lines that do not parse are
/// skipped; no file = none).
pub fn history(dir: &Path) -> Vec<Entry> {
    std::fs::read_to_string(dir.join(HISTORY_FILE))
        .map(|t| {
            t.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// When `history.jsonl` was last written (`None`: no file).
pub fn history_modified(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join(HISTORY_FILE))
        .and_then(|m| m.modified())
        .ok()
}

/// The running gateway's status; `None` when no gateway runs: no file, or
/// nobody holds `gateway.lock` (a gateway that crashed leaves its file, and
/// its pid may since belong to another process).
pub fn read_status(dir: &Path) -> Option<Status> {
    let status: Status =
        serde_json::from_slice(&std::fs::read(dir.join(STATUS_FILE)).ok()?).ok()?;
    gateway_running(dir).then_some(status)
}

/// When `status.json` was last written (`None`: no file): a reader polling
/// the status re-reads it only when this changes.
pub fn status_modified(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join(STATUS_FILE))
        .and_then(|m| m.modified())
        .ok()
}

/// A gateway holds `gateway.lock` right now (a non-blocking `flock` probe
/// that fails while it is held; the probe's own lock is dropped at once).
pub fn gateway_running(dir: &Path) -> bool {
    // read-only: probing never creates the file
    let Ok(f) = File::open(dir.join(LOCK_FILE)) else {
        return false;
    };
    // SAFETY: flock on a descriptor we own; closing it releases the lock
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    r != 0 && io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock
}

pub(crate) fn write_status(dir: &Path, status: &Status) -> io::Result<()> {
    write_json(dir, STATUS_FILE, status)
}

pub(crate) fn remove_status(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(STATUS_FILE));
}

/// Take `gateway.lock` (one gateway per user): `None` while another
/// process holds it. The lock lasts as long as the returned file. A
/// [`gateway_running`] probe holds it for an instant, so a busy lock is
/// tried again a few times before it counts as another gateway's.
pub(crate) fn lock_gateway(dir: &Path) -> io::Result<Option<File>> {
    ensure_dir(dir)?;
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(dir.join(LOCK_FILE))?;
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: flock on a descriptor we own
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(f));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::WouldBlock {
            return Err(e);
        }
    }
    Ok(None)
}

/// The device whose key is `key`.
pub(crate) fn find_device<'a>(all: &'a [Device], key: &[u8; KEY_LEN]) -> Option<&'a Device> {
    all.iter().find(|d| d.key_bytes().as_ref() == Some(key))
}

/// Spend the pairing secret `secret` (base64url) and record a device with
/// `key`: `None` when the secret is not live (unknown, used, expired).
pub(crate) fn pair_device(
    dir: &Path,
    secret: &str,
    name: &str,
    platform: &str,
    key: &[u8; KEY_LEN],
    now: SystemTime,
) -> io::Result<Option<Device>> {
    let Some(bytes) = B64.decode(secret.as_bytes()).ok() else {
        return Ok(None);
    };
    let hash = hash_secret(&bytes);
    let now = whole_secs(now);
    locked(dir, || {
        let mut live: Vec<Secret> = read_list(&dir.join(PAIRING_FILE))?;
        let before = live.len();
        live.retain(|s| s.expires_at > now);
        let found = live
            .iter()
            .position(|s| ct_eq(s.hash.as_bytes(), hash.as_bytes()));
        let permission = found.map(|i| live.remove(i).permission);
        if live.len() != before {
            write_json(dir, PAIRING_FILE, &live)?;
        }
        let Some(permission) = permission else {
            return Ok(None);
        };
        let mut all = devices(dir)?;
        // a key paired again replaces its old entry
        all.retain(|d| d.key_bytes().as_ref() != Some(key));
        let id = loop {
            let id = hex(&transport::random_bytes::<4>()?);
            if all.iter().all(|d| d.id != id) {
                break id;
            }
        };
        let device = Device {
            id,
            name: clean_text(name),
            platform: clean_text(platform),
            key: B64.encode(key),
            permission,
            paired_at: now,
            last_seen: now,
            push: None,
            ntfy: None,
        };
        all.push(device.clone());
        write_json(dir, DEVICES_FILE, &all)?;
        Ok(Some(device))
    })
}

/// Remove the device `id` (its own `forget`).
pub(crate) fn forget(dir: &Path, id: &str) -> io::Result<bool> {
    revoke(dir, id)
}

/// Record that the device `id` was seen at `now`.
pub(crate) fn touch(dir: &Path, id: &str, now: SystemTime) -> io::Result<()> {
    locked(dir, || {
        let mut all = devices(dir)?;
        let Some(d) = all.iter_mut().find(|d| d.id == id) else {
            return Ok(());
        };
        d.last_seen = whole_secs(now);
        write_json(dir, DEVICES_FILE, &all)
    })
}

/// Whether `c` may be shown from text a device sent (its name, a command it
/// ran): no control character and no bidirectional formatting character,
/// which would reorder what the desktop shows around it (a toast saying
/// `ls` about a command that is not `ls`).
pub fn shown_char(c: char) -> bool {
    !c.is_control()
        && !matches!(
            c,
            '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
}

/// Device-supplied text for display: control and bidirectional formatting
/// characters dropped ([`shown_char`]), at most [`MAX_NAME`] characters.
pub(crate) fn clean_text(s: &str) -> String {
    let out: String = s
        .chars()
        .filter(|&c| shown_char(c))
        .take(MAX_NAME)
        .collect();
    let out = out.trim().to_string();
    if out.is_empty() {
        "unnamed".into()
    } else {
        out
    }
}

fn hash_secret(secret: &[u8]) -> String {
    hex(&Sha256::digest(secret))
}

/// Constant-time equality (the length may leak; the contents do not).
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A base64url (no padding) 32-byte key.
pub fn decode_key(text: &str) -> Option<[u8; KEY_LEN]> {
    B64.decode(text.as_bytes()).ok()?.try_into().ok()
}

/// A key as base64url (no padding), as links and `devices.json` carry it.
pub fn encode_key(key: &[u8; KEY_LEN]) -> String {
    B64.encode(key)
}

/// Percent-encode everything but RFC 3986 unreserved characters.
pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Addresses a phone can dial for a gateway listening on `listen`: for an
/// unspecified address (`0.0.0.0`) every up, non-loopback, non-link-local
/// IPv4 interface address (none when there is no network); otherwise
/// `listen` itself.
pub fn addresses(listen: SocketAddr) -> Vec<SocketAddr> {
    if !listen.ip().is_unspecified() {
        return vec![listen];
    }
    ipv4_addrs()
        .into_iter()
        .map(|ip| SocketAddr::new(IpAddr::V4(ip), listen.port()))
        .collect()
}

fn ipv4_addrs() -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a linked list we only read, then free once
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut cur = list;
        while let Some(ifa) = cur.as_ref() {
            let up = ifa.ifa_flags & libc::IFF_UP as libc::c_uint != 0;
            if up
                && let Some(sa) = ifa.ifa_addr.as_ref()
                && i32::from(sa.sa_family) == libc::AF_INET
            {
                let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                if !ip.is_loopback() && !ip.is_link_local() && !out.contains(&ip) {
                    out.push(ip);
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    out
}

/// The QR code of `text` as rows of modules, `true` = dark, without the
/// quiet zone.
pub fn qr_matrix(text: &str) -> Vec<Vec<bool>> {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return Vec::new();
    };
    let w = code.width();
    let colors = code.to_colors();
    colors
        .chunks(w)
        .map(|row| row.iter().map(|c| *c == qrcode::Color::Dark).collect())
        .collect()
}

/// Modules of quiet zone around [`qr_terminal`]'s code.
pub const QR_QUIET: usize = 4;

/// The QR code of `text` for a terminal: two module rows per text row in
/// Unicode half blocks, light modules drawn in bright white on black (so it
/// scans on light and dark themes alike), with a [`QR_QUIET`]-module quiet
/// zone.
pub fn qr_terminal(text: &str) -> String {
    let m = qr_matrix(text);
    if m.is_empty() {
        return String::new();
    }
    let size = m.len() + 2 * QR_QUIET;
    let light = |r: usize, c: usize| -> bool {
        let (Some(r), Some(c)) = (r.checked_sub(QR_QUIET), c.checked_sub(QR_QUIET)) else {
            return true;
        };
        m.get(r).and_then(|row| row.get(c)).is_none_or(|dark| !dark)
    };
    let mut out = String::new();
    for r in (0..size).step_by(2) {
        out.push_str("\x1b[97;40m");
        for c in 0..size {
            let top = light(r, c);
            let bottom = r + 1 >= size || light(r + 1, c);
            out.push(match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `t` cut to whole seconds, as the files store it.
pub(crate) fn whole_secs(t: SystemTime) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(unix_secs(t))
}

/// `YYYY-MM-DDTHH:MM:SSZ` (UTC).
pub fn format_rfc3339(t: SystemTime) -> String {
    let secs = unix_secs(t);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// An RFC 3339 time (`Z` or a `±HH:MM` offset; fractions ignored).
pub fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let t = s.get(r)?;
        t.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| t.parse().ok())?
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if b[13] != b':' || b[16] != b':' || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    if h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let n = frac.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        rest = &frac[n..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        o if o.len() == 6 && matches!(&o[..1], "+" | "-") && &o[3..4] == ":" => {
            let oh: i64 = o[1..3].parse().ok()?;
            let om: i64 = o[4..6].parse().ok()?;
            let v = oh * 3600 + om * 60;
            if &o[..1] == "-" { -v } else { v }
        }
        _ => return None,
    };
    // days-from-civil (Howard Hinnant)
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset;
    u64::try_from(secs)
        .ok()
        .map(|s| UNIX_EPOCH + Duration::from_secs(s))
}

/// serde for `SystemTime` as RFC 3339 text.
mod rfc3339 {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::SystemTime;

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::format_rfc3339(*t))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let text = String::deserialize(d)?;
        super::parse_rfc3339(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("not an RFC 3339 time: {text:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "rift-registry-{name}-{}-{}",
            std::process::id(),
            hex(&transport::random_bytes::<4>().unwrap())
        ));
        remote_dir(&d)
    }

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().mode() & 0o777
    }

    /// The query of a pairing link as (key, value) pairs, in order.
    fn query(url: &str) -> Vec<(String, String)> {
        let q = url.strip_prefix("rift-remote://pair?").expect(url);
        q.split('&')
            .map(|kv| {
                let (k, v) = kv.split_once('=').expect(kv);
                (k.to_string(), v.to_string())
            })
            .collect()
    }

    #[test]
    fn files_are_private_and_written_atomically() {
        let dir = scratch("modes");
        let key = gateway_public_key(&dir).unwrap();
        assert_eq!(gateway_public_key(&dir).unwrap(), key, "created once");
        let private = std::fs::read(dir.join(KEY_FILE)).unwrap();
        assert_eq!(private.len(), 32);
        assert_eq!(
            transport::public_key(&private.try_into().unwrap()),
            key,
            "the file holds the private key"
        );
        let link = start_pairing(&dir, &[], "h", Permission::Operate).unwrap();
        assert!(link.url.contains("&k="));
        let secret = query(&link.url)[2].1.clone();
        let dev = pair_device(&dir, &secret, "n", "ios", &[7; 32], SystemTime::now())
            .unwrap()
            .unwrap();
        write_status(
            &dir,
            &Status {
                pid: std::process::id(),
                addresses: vec![],
                clients: vec![],
                relay: None,
            },
        )
        .unwrap();
        relay_secret(&dir).unwrap();
        append_history(&dir, &entry(0)).unwrap();
        let lock = lock_gateway(&dir).unwrap().unwrap();
        assert_eq!(mode(&dir), 0o700);
        for f in [
            KEY_FILE,
            DEVICES_FILE,
            PAIRING_FILE,
            STATUS_FILE,
            LOCK_FILE,
            RELAY_FILE,
            HISTORY_FILE,
        ] {
            assert_eq!(mode(&dir.join(f)), 0o600, "{f}");
        }
        // a loosened directory is tightened again on the next write
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        touch(&dir, &dev.id, SystemTime::now()).unwrap();
        assert_eq!(mode(&dir), 0o700);
        // atomic: the file is replaced (new inode), never truncated in place,
        // and no temporary file is left behind
        let before = std::fs::metadata(dir.join(DEVICES_FILE)).unwrap().ino();
        touch(&dir, &dev.id, SystemTime::now()).unwrap();
        let after = std::fs::metadata(dir.join(DEVICES_FILE)).unwrap().ino();
        assert_ne!(before, after);
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        drop(lock);
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn concurrent_writers_lose_nothing() {
        let dir = scratch("race");
        gateway_public_key(&dir).unwrap();
        let secrets: Vec<String> = (0..16)
            .map(|_| {
                let l = start_pairing(&dir, &[], "h", Permission::View).unwrap();
                query(&l.url)[2].1.clone()
            })
            .collect();
        let threads: Vec<_> = secrets
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    pair_device(&dir, &s, "n", "ios", &[i as u8; 32], SystemTime::now())
                        .unwrap()
                        .unwrap()
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(devices(&dir).unwrap().len(), 16);
        assert_eq!(read_list::<Secret>(&dir.join(PAIRING_FILE)).unwrap(), []);
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn pairing_link_has_the_fixture_shape() {
        let dir = scratch("link");
        let addrs: Vec<SocketAddr> = vec![
            "192.168.1.29:7878".parse().unwrap(),
            "10.0.0.5:7878".parse().unwrap(),
        ];
        // expiry is in whole seconds, from the second pairing started in
        let before = whole_secs(SystemTime::now());
        let link = start_pairing(&dir, &addrs, "studio-mac", Permission::Operate).unwrap();
        // a loaded machine can take seconds over the call: allow what it took
        let took = SystemTime::now().duration_since(before).unwrap();
        let fixture = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/pairing-link.txt"),
        )
        .unwrap();
        let (got, want) = (query(&link.url), query(fixture.trim()));
        let keys = |q: &[(String, String)]| q.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&got), keys(&want));
        assert_eq!(keys(&got), ["a", "k", "s", "n", "p"]);
        for (g, w) in got.iter().zip(&want) {
            match g.0.as_str() {
                // random per link: same encoding and length
                "k" | "s" => {
                    assert_eq!(g.1.len(), w.1.len(), "{}", g.0);
                    assert!(B64.decode(&g.1).is_ok() && B64.decode(&w.1).is_ok());
                }
                _ => assert_eq!(g, w),
            }
        }
        assert_eq!(
            decode_key(&got[1].1),
            Some(gateway_public_key(&dir).unwrap())
        );
        assert_eq!(B64.decode(&got[2].1).unwrap().len(), SECRET_LEN);
        let ttl = link.expires_at.duration_since(before).unwrap();
        assert!(ttl >= PAIRING_TTL && ttl <= PAIRING_TTL + took + Duration::from_secs(1));
        // the stored hash, never the secret
        let stored = std::fs::read_to_string(dir.join(PAIRING_FILE)).unwrap();
        assert!(!stored.contains(&got[2].1));
        assert!(stored.contains(&hash_secret(&B64.decode(&got[2].1).unwrap())));
        // a host name that needs escaping
        let l = start_pairing(&dir, &addrs, "Sam's Mac & co", Permission::View).unwrap();
        assert!(
            l.url.ends_with("&n=Sam%27s%20Mac%20%26%20co&p=view"),
            "{}",
            l.url
        );
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn secrets_are_one_use_and_expire() {
        let dir = scratch("secret");
        let link = start_pairing(&dir, &[], "h", Permission::View).unwrap();
        let s = query(&link.url)[2].1.clone();
        let now = SystemTime::now();
        // expired
        assert_eq!(
            pair_device(&dir, &s, "a", "ios", &[1; 32], link.expires_at).unwrap(),
            None
        );
        let link = start_pairing(&dir, &[], "h", Permission::View).unwrap();
        let s = query(&link.url)[2].1.clone();
        assert_eq!(
            pair_device(&dir, "AAAAAAAAAAAAAAAAAAAAAA", "a", "ios", &[1; 32], now).unwrap(),
            None,
            "unknown"
        );
        assert_eq!(
            pair_device(&dir, "not base64!", "a", "ios", &[1; 32], now).unwrap(),
            None
        );
        let d = pair_device(&dir, &s, "a\u{7}\nb", "ios", &[1; 32], now)
            .unwrap()
            .unwrap();
        assert_eq!(d.permission, Permission::View);
        assert_eq!(d.name, "ab");
        assert_eq!(d.id.len(), 8);
        assert_eq!(
            pair_device(&dir, &s, "a", "ios", &[2; 32], now).unwrap(),
            None,
            "spent"
        );
        assert_eq!(find_device(&devices(&dir).unwrap(), &[1; 32]), Some(&d));
        assert!(revoke(&dir, &d.id).unwrap());
        assert!(!revoke(&dir, &d.id).unwrap());
        assert_eq!(devices(&dir).unwrap(), []);
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn corrupt_devices_file_is_an_error_not_empty() {
        let dir = scratch("corrupt");
        assert_eq!(devices(&dir).unwrap(), [], "missing = none");
        ensure_dir(&dir).unwrap();
        std::fs::write(dir.join(DEVICES_FILE), b"[{").unwrap();
        assert_eq!(
            devices(&dir).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn status_needs_the_held_lock_and_the_lock_is_exclusive() {
        let dir = scratch("status");
        assert_eq!(read_status(&dir), None);
        assert!(!gateway_running(&dir));
        assert!(!dir.join(LOCK_FILE).exists(), "probing creates nothing");
        let held = lock_gateway(&dir).unwrap();
        assert!(held.is_some());
        assert!(gateway_running(&dir));
        let st = Status {
            pid: std::process::id(),
            addresses: vec!["127.0.0.1:7878".parse().unwrap()],
            clients: vec![Client {
                device: "d3f1".into(),
                name: "phone".into(),
                since: UNIX_EPOCH + Duration::from_secs(1_760_000_000),
                via: Via::Relay,
            }],
            relay: Some(RelayStatus {
                url: "wss://relay.example.com".into(),
                state: RelayState::Error,
                error: Some("the relay is full".into()),
            }),
        };
        write_status(&dir, &st).unwrap();
        assert_eq!(read_status(&dir), Some(st.clone()));
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(STATUS_FILE)).unwrap()).unwrap();
        assert_eq!(json["clients"][0]["since"], "2025-10-09T08:53:20Z");
        assert_eq!(json["clients"][0]["via"], "relay");
        assert_eq!(
            json["relay"],
            serde_json::json!({"url":"wss://relay.example.com","state":"error","error":"the relay is full"})
        );
        assert_eq!(json["addresses"][0], "127.0.0.1:7878");
        // a status without a relay has no `relay` key; a client without
        // `via` (older gateway) reads as lan
        let plain: Status = serde_json::from_str(
            r#"{"pid":1,"addresses":[],"clients":[{"device":"a","name":"b","since":"2026-10-12T10:00:00Z"}]}"#,
        )
        .unwrap();
        assert_eq!((&plain.relay, plain.clients[0].via), (&None, Via::Lan));
        assert!(!serde_json::to_string(&plain).unwrap().contains("relay"));
        assert!(
            lock_gateway(&dir).unwrap().is_none(),
            "one gateway per user"
        );
        // the gateway crashed and its pid was reused (here: by this live
        // process): the file stays, the lock is gone, so no gateway runs
        drop(held);
        assert!(dir.join(STATUS_FILE).exists());
        assert_eq!(read_status(&dir), None);
        assert!(!gateway_running(&dir));
        // a probe never keeps the lock: a gateway can still take it
        let again = lock_gateway(&dir).unwrap();
        assert!(again.is_some());
        assert_eq!(read_status(&dir), Some(st));
        drop(again);
        remove_status(&dir);
        assert!(!dir.join(STATUS_FILE).exists());
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    fn entry(i: u64) -> Entry {
        Entry {
            device: i.is_multiple_of(2).then(|| format!("d{i}")),
            name: i.is_multiple_of(2).then(|| format!("Phone {i}")),
            via: if i.is_multiple_of(3) {
                Via::Relay
            } else {
                Via::Lan
            },
            address: format!("10.0.0.{}:5000", i % 250),
            start: UNIX_EPOCH + Duration::from_secs(1_760_000_000 + i),
            end: UNIX_EPOCH + Duration::from_secs(1_760_000_060 + i),
            reason: "closed".into(),
        }
    }

    #[test]
    fn history_keeps_the_newest_500_and_rewrites_atomically() {
        let dir = scratch("history");
        assert_eq!(history(&dir), []);
        append_history(&dir, &entry(0)).unwrap();
        let line = std::fs::read_to_string(dir.join(HISTORY_FILE)).unwrap();
        assert_eq!(
            line,
            "{\"device\":\"d0\",\"name\":\"Phone 0\",\"via\":\"relay\",\"address\":\"10.0.0.0:5000\",\
             \"start\":\"2025-10-09T08:53:20Z\",\"end\":\"2025-10-09T08:54:20Z\",\"reason\":\"closed\"}\n"
        );
        // a failed handshake: no device
        append_history(&dir, &entry(1)).unwrap();
        assert!(
            std::fs::read_to_string(dir.join(HISTORY_FILE))
                .unwrap()
                .contains("{\"device\":null,\"name\":null,\"via\":\"lan\"")
        );
        let ino = || std::fs::metadata(dir.join(HISTORY_FILE)).unwrap().ino();
        let first = ino();
        for i in 2..MAX_HISTORY as u64 {
            append_history(&dir, &entry(i)).unwrap();
        }
        assert_eq!(ino(), first, "appended in place while there is room");
        assert_eq!(history(&dir).len(), MAX_HISTORY);
        append_history(&dir, &entry(MAX_HISTORY as u64)).unwrap();
        assert_ne!(ino(), first, "rewritten (new inode) when full");
        let all = history(&dir);
        assert_eq!(all.len(), MAX_HISTORY);
        // the oldest failed handshake went, not the older device entry
        assert_eq!((&all[0], &all[1]), (&entry(0), &entry(2)));
        assert_eq!(all[MAX_HISTORY - 1], entry(MAX_HISTORY as u64));
        assert_eq!(mode(&dir.join(HISTORY_FILE)), 0o600);
        // a torn or foreign line is skipped, the rest still reads
        let mut f = OpenOptions::new()
            .append(true)
            .open(dir.join(HISTORY_FILE))
            .unwrap();
        f.write_all(b"{\"device\":\n").unwrap();
        assert_eq!(history(&dir).len(), MAX_HISTORY);
        append_history(&dir, &entry(9999)).unwrap();
        let all = history(&dir);
        assert_eq!(all.last(), Some(&entry(9999)));
        assert!(all.len() <= MAX_HISTORY);
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn a_flood_of_failed_handshakes_never_pushes_out_device_entries() {
        let dir = scratch("history-flood");
        let devices: Vec<Entry> = (0..30).map(|i| entry(i * 2)).collect();
        for e in &devices {
            append_history(&dir, e).unwrap();
        }
        // anyone can cause refused attempts: more than the history holds
        for i in 0..MAX_HISTORY as u64 + 100 {
            append_history(&dir, &entry(10_001 + i * 2)).unwrap();
        }
        let all = history(&dir);
        assert_eq!(all.len(), MAX_HISTORY);
        assert_eq!(&all[..30], &devices[..]);
        assert!(all[30..].iter().all(|e| e.device.is_none()));
        // with only device entries left to drop, the oldest goes
        for i in 0..MAX_HISTORY as u64 {
            append_history(&dir, &entry(20_000 + i * 2)).unwrap();
        }
        let all = history(&dir);
        assert_eq!(all.len(), MAX_HISTORY);
        assert!(all.iter().all(|e| e.device.is_some()));
        assert_eq!(all[0], entry(20_000));
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn permissions_change_in_place() {
        let dir = scratch("permission");
        let link = start_pairing(&dir, &[], "h", Permission::Operate).unwrap();
        let s = query(&link.url)[2].1.clone();
        let d = pair_device(&dir, &s, "a", "ios", &[3; 32], SystemTime::now())
            .unwrap()
            .unwrap();
        assert!(set_permission(&dir, &d.id, Permission::View).unwrap());
        assert_eq!(devices(&dir).unwrap()[0].permission, Permission::View);
        assert!(
            set_permission(&dir, &d.id, Permission::View).unwrap(),
            "unchanged"
        );
        assert!(set_permission(&dir, &d.id, Permission::Operate).unwrap());
        assert_eq!(devices(&dir).unwrap()[0].permission, Permission::Operate);
        assert!(!set_permission(&dir, "nope", Permission::View).unwrap());
        // everything else about the device stays
        let after = devices(&dir).unwrap()[0].clone();
        assert_eq!((after.id, after.key, after.name), (d.id, d.key, d.name));
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn push_registrations_keep_failures_only_and_older_files_default_it() {
        let dir = scratch("push");
        let link = start_pairing(&dir, &[], "h", Permission::View).unwrap();
        let s = query(&link.url)[2].1.clone();
        let d = pair_device(&dir, &s, "a", "ios", &[4; 32], SystemTime::now())
            .unwrap()
            .unwrap();
        let reg = PushReg {
            token: Some("ExponentPushToken[x]".into()),
            ntfy: None,
            long_ms: Some(30_000),
            show_command: false,
            failures_only: true,
        };
        assert!(set_push(&dir, &d.id, Some(reg.clone())).unwrap());
        assert_eq!(devices(&dir).unwrap()[0].alerts().as_ref(), Some(&reg));
        let mut file: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(DEVICES_FILE)).unwrap()).unwrap();
        assert_eq!(file[0]["push"]["failures_only"], true, "{file}");
        // a Phase 2 file has no `failures_only`: every long command alerts
        file[0]["push"]
            .as_object_mut()
            .unwrap()
            .remove("failures_only");
        std::fs::write(dir.join(DEVICES_FILE), file.to_string()).unwrap();
        let read = devices(&dir).unwrap()[0].push.clone().unwrap();
        assert!(!read.failures_only);
        assert_eq!(read.long_ms, Some(30_000));
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn push_registrations_keep_an_ntfy_topic_and_older_files_have_none() {
        let dir = scratch("ntfy");
        let link = start_pairing(&dir, &[], "h", Permission::View).unwrap();
        let s = query(&link.url)[2].1.clone();
        let d = pair_device(&dir, &s, "a", "ios", &[5; 32], SystemTime::now())
            .unwrap()
            .unwrap();
        // ntfy alone, no Expo token
        let reg = PushReg {
            token: None,
            ntfy: Some("https://ntfy.sh/rift-abc".into()),
            long_ms: None,
            show_command: true,
            failures_only: false,
        };
        assert!(set_push(&dir, &d.id, Some(reg.clone())).unwrap());
        assert_eq!(devices(&dir).unwrap()[0].alerts().as_ref(), Some(&reg));
        let mut file: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(DEVICES_FILE)).unwrap()).unwrap();
        assert_eq!(file[0]["ntfy"]["url"], "https://ntfy.sh/rift-abc", "{file}");
        assert!(file[0].get("push").is_none(), "no Expo half: {file}");
        // a file written before ntfy: a token string and no `ntfy`
        file[0]["push"] = serde_json::json!({
            "token": "ExponentPushToken[x]",
            "long_ms": 1000,
            "show_command": false,
        });
        file[0].as_object_mut().unwrap().remove("ntfy");
        std::fs::write(dir.join(DEVICES_FILE), file.to_string()).unwrap();
        let read = devices(&dir).unwrap()[0].alerts().unwrap();
        assert_eq!(read.token.as_deref(), Some("ExponentPushToken[x]"));
        assert_eq!(read.ntfy, None);
        assert_eq!(read.long_ms, Some(1000));
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    /// `devices.json` as Rift 373fa25 (before ntfy) reads it: `push.token`
    /// is a required string.
    mod at_373fa25 {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        pub struct PushReg {
            pub token: String,
            pub long_ms: Option<u64>,
            pub show_command: bool,
        }
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        pub struct Device {
            pub id: String,
            pub name: String,
            pub platform: String,
            pub key: String,
            pub permission: crate::protocol::Permission,
            pub paired_at: String,
            pub last_seen: String,
            #[serde(default)]
            pub push: Option<PushReg>,
        }
    }

    #[test]
    fn an_older_rift_reads_the_devices_file_with_any_registration() {
        let dir = scratch("downgrade");
        let mut ids = Vec::new();
        for (i, key) in [[6; 32], [7; 32], [8; 32]].iter().enumerate() {
            let link = start_pairing(&dir, &[], "h", Permission::View).unwrap();
            let s = query(&link.url)[2].1.clone();
            let d = pair_device(&dir, &s, &format!("d{i}"), "ios", key, SystemTime::now())
                .unwrap()
                .unwrap();
            ids.push(d.id);
        }
        let reg = |token: Option<&str>, ntfy: Option<&str>| PushReg {
            token: token.map(Into::into),
            ntfy: ntfy.map(Into::into),
            long_ms: Some(1000),
            show_command: false,
            failures_only: true,
        };
        set_push(&dir, &ids[0], Some(reg(None, Some("https://ntfy.sh/a")))).unwrap();
        set_push(
            &dir,
            &ids[1],
            Some(reg(Some("ExponentPushToken[x]"), Some("https://ntfy.sh/b"))),
        )
        .unwrap();
        set_push(&dir, &ids[2], Some(reg(Some("ExponentPushToken[y]"), None))).unwrap();
        let bytes = std::fs::read(dir.join(DEVICES_FILE)).unwrap();
        let old: Vec<at_373fa25::Device> = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("373fa25 cannot read it: {e}"));
        assert_eq!(old.len(), 3);
        assert!(
            old[0].push.is_none(),
            "ntfy only: no Expo half for an old build"
        );
        assert_eq!(old[1].push.as_ref().unwrap().token, "ExponentPushToken[x]");
        assert_eq!(old[2].push.as_ref().unwrap().token, "ExponentPushToken[y]");
        // and the new code reads all of it back
        let now = devices(&dir).unwrap();
        assert_eq!(
            now[1].alerts(),
            Some(reg(Some("ExponentPushToken[x]"), Some("https://ntfy.sh/b")))
        );
        // an unregistered token goes; the ntfy half stays
        assert!(clear_token(&dir, &ids[1], "ExponentPushToken[x]").unwrap());
        assert!(!clear_token(&dir, &ids[1], "ExponentPushToken[x]").unwrap());
        assert!(!clear_token(&dir, &ids[2], "ExponentPushToken[other]").unwrap());
        let now = devices(&dir).unwrap();
        assert_eq!(now[1].alerts(), Some(reg(None, Some("https://ntfy.sh/b"))));
        assert!(now[2].push.is_some());
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn the_interim_push_shape_with_a_null_token_still_loads() {
        let dir = scratch("interim");
        std::fs::create_dir_all(&dir).unwrap();
        let file = r#"[
          {"id":"a1","name":"p","platform":"ios","key":"k","permission":"operate",
           "paired_at":"2026-10-13T05:59:30Z","last_seen":"2026-10-13T10:08:24Z",
           "push":{"token":null,"ntfy":"https://ntfy.sh/t","long_ms":10000,
                   "show_command":false,"failures_only":true}},
          {"id":"b2","name":"q","platform":"android","key":"k2","permission":"view",
           "paired_at":"2026-10-13T05:59:30Z","last_seen":"2026-10-13T10:08:24Z"}
        ]"#;
        std::fs::write(dir.join(DEVICES_FILE), file).unwrap();
        let list = devices(&dir).unwrap_or_else(|e| panic!("interim file refused: {e}"));
        assert_eq!(list.len(), 2);
        assert!(list[0].push.is_none());
        assert_eq!(
            list[0].alerts(),
            Some(PushReg {
                token: None,
                ntfy: Some("https://ntfy.sh/t".into()),
                long_ms: Some(10000),
                show_command: false,
                failures_only: true,
            })
        );
        assert_eq!(list[1].alerts(), None);
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn relay_secret_and_routing_id_are_stable() {
        let dir = scratch("relay");
        let s = relay_secret(&dir).unwrap();
        assert_eq!(relay_secret(&dir).unwrap(), s, "created once");
        assert_ne!(s, [0; RELAY_SECRET_LEN]);
        let stored = std::fs::read_to_string(dir.join(RELAY_FILE)).unwrap();
        assert!(stored.contains(&B64.encode(s)));
        std::fs::write(dir.join(RELAY_FILE), b"{\"secret\":\"short\"}").unwrap();
        assert_eq!(
            relay_secret(&dir).unwrap_err().kind(),
            io::ErrorKind::InvalidData,
            "a damaged file is an error, never a new id"
        );
        // base64url of the first 16 bytes of SHA-256 over the public key
        let id = routing_id(&[0u8; 32]);
        assert_eq!(id.len(), 22);
        assert_eq!(
            B64.decode(&id).unwrap(),
            Sha256::digest([0u8; 32])[..ROUTING_ID_LEN].to_vec()
        );
        assert!(crate::relay::valid_id(&id));
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn relay_pairing_link_has_the_fixture_shape() {
        let dir = scratch("link-relay");
        let addrs: Vec<SocketAddr> = vec!["192.168.1.29:7878".parse().unwrap()];
        let link = start_pairing_via(
            &dir,
            &addrs,
            Some("wss://relay.example.com"),
            "studio-mac",
            Permission::Operate,
        )
        .unwrap();
        let fixture = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/pairing-link-relay.txt"),
        )
        .unwrap();
        let (got, want) = (query(&link.url), query(fixture.trim()));
        let keys = |q: &[(String, String)]| q.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&got), ["a", "k", "s", "n", "p", "r"]);
        assert_eq!(keys(&got), keys(&want));
        for (g, w) in got.iter().zip(&want) {
            if !matches!(g.0.as_str(), "k" | "s") {
                assert_eq!(g, w);
            }
        }
        // no relay (or an empty one): no `r`
        for relay in [None, Some("")] {
            let l = start_pairing_via(&dir, &addrs, relay, "h", Permission::View).unwrap();
            assert!(!l.url.contains("&r="), "{}", l.url);
        }
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn rfc3339_round_trips() {
        for secs in [0u64, 951_782_400, 1_760_000_000, 4_102_444_800] {
            let t = UNIX_EPOCH + Duration::from_secs(secs);
            assert_eq!(parse_rfc3339(&format_rfc3339(t)), Some(t), "{secs}");
        }
        assert_eq!(format_rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            format_rfc3339(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            parse_rfc3339("2026-10-12T12:00:00.123+02:00"),
            parse_rfc3339("2026-10-12T10:00:00Z")
        );
        for bad in [
            "",
            "2026-10-12",
            "2026-13-01T00:00:00Z",
            "2026-10-12T00:00:00",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn addresses_are_the_listen_address_or_lan_interfaces() {
        let lo: SocketAddr = "127.0.0.1:17878".parse().unwrap();
        assert_eq!(addresses(lo), [lo]);
        let any: SocketAddr = "0.0.0.0:17878".parse().unwrap();
        let all = addresses(any);
        assert!(all.iter().all(|a| a.port() == 17878));
        assert!(
            all.iter().all(|a| match a.ip() {
                IpAddr::V4(ip) => !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified(),
                IpAddr::V6(_) => false,
            }),
            "{all:?}"
        );
    }

    #[test]
    fn qr_codes_decode_to_a_square_with_finder_patterns() {
        let url = "rift-remote://pair?a=192.168.1.29:7878,10.0.0.5:7878&k=3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&s=q83vEjRWeJCrze8SNFZ4kA&n=studio-mac&p=operate";
        let m = qr_matrix(url);
        let n = m.len();
        assert!(n >= 21 && (n - 21).is_multiple_of(4), "{n}");
        assert!(m.iter().all(|r| r.len() == n));
        // the three finder patterns: a dark 7×7 ring around a light ring
        for (r0, c0) in [(0, 0), (0, n - 7), (n - 7, 0)] {
            for i in 0..7 {
                assert!(m[r0][c0 + i] && m[r0 + 6][c0 + i] && m[r0 + i][c0] && m[r0 + i][c0 + 6]);
            }
            assert!(!m[r0 + 1][c0 + 1] && m[r0 + 3][c0 + 3]);
        }
        let t = qr_terminal(url);
        let rows: Vec<&str> = t.lines().collect();
        assert_eq!(rows.len(), (n + 2 * QR_QUIET).div_ceil(2));
        let first = rows[0]
            .trim_start_matches("\x1b[97;40m")
            .trim_end_matches("\x1b[0m");
        assert_eq!(first.chars().count(), n + 2 * QR_QUIET);
        assert!(first.chars().all(|c| c == '█'), "quiet zone is light");
    }

    #[test]
    fn device_names_lose_controls_and_bidi_overrides() {
        assert_eq!(clean_text("Jo\u{202E}enohP\u{202C}\n"), "JoenohP");
        assert_eq!(clean_text("a\u{2066}b\u{2069}\u{200F}c\u{61C}"), "abc");
        assert_eq!(clean_text("\u{202E}\u{1b}"), "unnamed");
        // other format characters (a joiner in an emoji) stay
        assert_eq!(clean_text("👩\u{200D}💻 Jo’s"), "👩\u{200D}💻 Jo’s");
    }
}
