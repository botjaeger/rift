//! In-app updates: the release manifest, the update check and the install.
//!
//! Releases are published to the public repository
//! `botjaeger/rift-releases`; each carries `latest.json` (written by
//! `scripts/release-manifest.py`):
//!
//! ```json
//! {"version": "0.2.0", "date": "2026-11-01", "protocol": 1,
//!  "notes": "…", "url": "https://github.com/…/releases/tag/v0.2.0",
//!  "assets": {"macos-aarch64": {"url": "…dmg", "sha256": "…", "kind": "dmg"},
//!             "macos-x86_64":  {"url": "…dmg", "sha256": "…", "kind": "dmg"},
//!             "linux-x86_64":  {"url": "…AppImage", "sha256": "…", "kind": "appimage"}}}
//! ```
//!
//! Next to it, `latest.json.sig` holds the base64 Ed25519 signature of
//! `latest.json`'s exact bytes, made with the maintainer's offline key
//! (`scripts/sign-manifest.sh`). The asset digests come from the same public
//! repository as the assets, so the signature is what ties a release to the
//! maintainer: a manifest whose signature does not verify against
//! [`RELEASE_KEYS`] is rejected before it is parsed.
//!
//! The check fetches both with the system `curl` on a background thread (no
//! HTTP stack in the binary), at most every [`CHECK_EVERY`], and caches the
//! signed manifest in `<data dir>/update.json` (re-verified whenever it is
//! read). Installing downloads this platform's
//! asset, verifies its SHA-256, swaps the app bundle (macOS) or AppImage
//! (Linux) in place, and leaves a marker (`update-pending.json`) that a
//! detached helper uses to roll back when the new version does not start.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the newest release's manifest lives (GitHub serves the newest
/// release's asset under `latest/download`).
pub const DEFAULT_URL: &str =
    "https://github.com/botjaeger/rift-releases/releases/latest/download/latest.json";
/// Overrides [`DEFAULT_URL`] (tests: a `file://` manifest).
pub const URL_ENV: &str = "RIFT_UPDATE_URL";
/// How often the check runs (and how old a cached answer may be at launch).
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Largest manifest accepted.
pub const MAX_MANIFEST: usize = 64 * 1024;
/// Largest download accepted (`curl --max-filesize`, and checked after the
/// download: curl cannot enforce it without a Content-Length).
pub const MAX_ASSET: u64 = 512 << 20;
/// Every asset URL must start with this: the public releases' downloads.
pub const DOWNLOAD_PREFIX: &str = "https://github.com/botjaeger/rift-releases/releases/download/";
/// Why a copy running from a disk image or a translocated folder cannot
/// update itself (also the dialog's text).
pub const MOVE_TO_APPLICATIONS: &str = "Move Rift to Applications to update it";

/// The Ed25519 public keys (base64 of the raw 32 bytes) whose signature on
/// `latest.json` is accepted. The private key never leaves the maintainer's
/// machine (`~/.config/rift-release/signing-key.pem`). A list so a key can
/// be rotated: ship a release that adds the new key, sign later releases
/// with it, and drop the old one once installs have moved on.
/// `scripts/sign-manifest.sh` reads this list to check its own signature.
pub const RELEASE_KEYS: &[&str] = &["NUQUzb/PNNa7VWjT6GJJuEz4FFPOG/3/h2ydgJmC+hY="];
/// An extra trusted key (base64, 32 bytes) for tests; only honoured while
/// the manifest comes from a `file://` URL (the [`URL_ENV`] test override),
/// never for https.
pub const TEST_KEY_ENV: &str = "RIFT_UPDATE_TEST_KEY";
/// The signature's URL is the manifest's with this appended.
pub const SIG_SUFFIX: &str = ".sig";
/// Largest signature file accepted (base64 of 64 bytes is 88 characters).
const MAX_SIG: usize = 1024;
/// The signature could not be fetched.
pub const SIG_MISSING: &str = "release signature is missing";
/// The signature does not verify against a trusted key.
pub const SIG_INVALID: &str = "release signature is invalid";

/// A raw Ed25519 public key from base64.
fn decode_key(b64: &str) -> Option<[u8; 32]> {
    crate::osc::base64_decode(b64.trim())?.try_into().ok()
}

/// The keys a manifest fetched from `url` may be signed with: the
/// [`RELEASE_KEYS`], plus `test_key` when `url` is a `file://` test manifest.
pub fn trusted_keys_for(url: &str, test_key: Option<&str>) -> Vec<[u8; 32]> {
    let mut keys: Vec<[u8; 32]> = RELEASE_KEYS.iter().filter_map(|k| decode_key(k)).collect();
    if url.starts_with("file://")
        && let Some(k) = test_key.and_then(decode_key)
    {
        keys.push(k);
    }
    keys
}

/// [`trusted_keys_for`] with [`TEST_KEY_ENV`].
pub fn trusted_keys(url: &str) -> Vec<[u8; 32]> {
    trusted_keys_for(url, std::env::var(TEST_KEY_ENV).ok().as_deref())
}

/// Whether `sig` (base64 of a 64-byte Ed25519 signature; whitespace is
/// ignored) signs exactly `body` with one of `keys`.
pub fn verify_signature(body: &[u8], sig: &str, keys: &[[u8; 32]]) -> Result<(), String> {
    let compact: String = sig.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let Some(sig) = crate::osc::base64_decode(&compact)
        .and_then(|b| <[u8; 64]>::try_from(b).ok())
        .map(ed25519_compact::Signature::new)
    else {
        return Err(SIG_INVALID.into());
    };
    if keys.iter().any(|k| {
        ed25519_compact::PublicKey::new(*k)
            .verify(body, &sig)
            .is_ok()
    }) {
        Ok(())
    } else {
        Err(SIG_INVALID.into())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    /// `dmg` or `appimage`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub date: String,
    /// The session-service protocol of that release (`ipc::PROTOCOL_VERSION`).
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub notes: String,
    /// The release page.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub assets: BTreeMap<String, Asset>,
}

/// What the last check found.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Status {
    /// Not checked (checks off, or a scripted run).
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Manifest),
    Failed(String),
}

/// `x.y.z` with an optional `-pre` (and ignored `+build`); a leading `v` is
/// allowed. Missing components count as 0 (`1.2` = `1.2.0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub nums: [u64; 3],
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            Some(_) => return None,
            None => (s, None),
        };
        let mut nums = [0u64; 3];
        let parts: Vec<&str> = core.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }
        for (i, p) in parts.iter().enumerate() {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            nums[i] = p.parse().ok()?;
        }
        if pre
            .as_deref()
            .is_some_and(|p| !p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'))
        {
            return None;
        }
        Some(Version { nums, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.nums.cmp(&other.nums).then_with(|| {
            // a release sorts after its pre-releases
            match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => compare_pre(a, b),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Dot-separated identifiers: numeric ones numerically and before
/// alphanumeric ones (semver §11).
fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut ia = a.split('.');
    let mut ib = b.split('.');
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// `candidate` is a strictly newer version than `current` (false when
/// either does not parse).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// `a` and `b` name the same version (`v0.2.0` = `0.2.0`); false when
/// either does not parse.
pub fn same_version(a: &str, b: &str) -> bool {
    matches!((Version::parse(a), Version::parse(b)), (Some(x), Some(y)) if x == y)
}

/// A version exactly as releases are tagged: `x.y.z` or `x.y.z-pre` (a
/// leading `v` allowed), nothing else. Stricter than [`Version::parse`]
/// (no `+build`, no missing components): it ends up in dialogs and paths.
pub fn valid_release_version(s: &str) -> bool {
    let s = s.strip_prefix('v').unwrap_or(s);
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (s, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| (1..=9).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
        && pre.is_none_or(|p| {
            (1..=64).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
        })
}

/// Validate a manifest body: JSON, bounded, a release version, asset URLs
/// in the public releases and SHA-256 digests in hex. `allow_file` admits
/// `file://` asset URLs (only when the manifest itself came from the test
/// override).
pub fn parse_manifest(body: &[u8], allow_file: bool) -> Result<Manifest, String> {
    if body.len() > MAX_MANIFEST {
        return Err("manifest too large".into());
    }
    let m: Manifest =
        serde_json::from_slice(body).map_err(|e| format!("malformed manifest: {e}"))?;
    validate(&m, allow_file)?;
    Ok(m)
}

/// The checks of [`parse_manifest`] (also run on the cached manifest).
pub fn validate(m: &Manifest, allow_file: bool) -> Result<(), String> {
    if !valid_release_version(&m.version) {
        return Err(format!("malformed manifest: version {:?}", m.version));
    }
    for (key, a) in &m.assets {
        if !asset_url_allowed(&a.url, allow_file) {
            return Err(format!("malformed manifest: {key} url {:?}", a.url));
        }
        if a.sha256.len() != 64 || !a.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("malformed manifest: {key} sha256"));
        }
        if a.kind != "dmg" && a.kind != "appimage" {
            return Err(format!("malformed manifest: {key} kind {:?}", a.kind));
        }
    }
    Ok(())
}

/// Where the manifest may come from: https, or a file under the test
/// override.
fn url_allowed(url: &str, allow_file: bool) -> bool {
    url.starts_with("https://") || (allow_file && url.starts_with("file://"))
}

/// Where an asset may come from: a plain path under [`DOWNLOAD_PREFIX`]
/// (no `..`, query or escapes curl would resolve elsewhere), or a file
/// under the test override.
fn asset_url_allowed(url: &str, allow_file: bool) -> bool {
    if allow_file && url.starts_with("file://") {
        return true;
    }
    url.strip_prefix(DOWNLOAD_PREFIX).is_some_and(|rest| {
        !rest.is_empty()
            && !rest.contains("..")
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
    })
}

/// What a fetched manifest means for the running `current` version.
pub fn evaluate(m: Manifest, current: &str) -> Status {
    if is_newer(&m.version, current) {
        Status::Available(m)
    } else {
        Status::UpToDate
    }
}

/// Manifest key of this build: `macos-aarch64`, `linux-x86_64`, ….
pub fn platform_key() -> String {
    asset_key(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn asset_key(os: &str, arch: &str) -> String {
    format!("{os}-{arch}")
}

/// This platform's asset, if the release has one.
pub fn asset_for<'a>(m: &'a Manifest, key: &str) -> Option<&'a Asset> {
    m.assets.get(key)
}

/// Whether the release's session service speaks `current` (running shells
/// can be reattached after the update). A manifest without the field is
/// treated as incompatible.
pub fn protocol_compatible(m: &Manifest, current: u32) -> bool {
    m.protocol == Some(current)
}

/// First paragraph of release notes, at most `max` characters.
pub fn summary(notes: &str, max: usize) -> String {
    let para = notes.split("\n\n").next().unwrap_or("").trim();
    let flat: String = para.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut s: String = flat.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// The manifest URL: `RIFT_UPDATE_URL`, else [`DEFAULT_URL`].
pub fn manifest_url() -> String {
    std::env::var(URL_ENV)
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_URL.into())
}

/// The test override is set.
pub fn url_overridden() -> bool {
    std::env::var_os(URL_ENV).is_some_and(|u| !u.is_empty())
}

// ── cache ──

/// A manifest whose signature verified, with the bytes it was verified on.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    pub manifest: Manifest,
    /// `latest.json` exactly as fetched.
    pub body: String,
    /// `latest.json.sig` (base64).
    pub signature: String,
}

/// Verify `sig` over `body` with `keys`, then parse and validate `body`.
pub fn open_signed(
    body: &[u8],
    sig: &str,
    keys: &[[u8; 32]],
    allow_file: bool,
) -> Result<Fetched, String> {
    if body.len() > MAX_MANIFEST {
        return Err("manifest too large".into());
    }
    verify_signature(body, sig, keys)?;
    let manifest = parse_manifest(body, allow_file)?;
    let body = String::from_utf8(body.to_vec()).map_err(|_| "malformed manifest: not UTF-8")?;
    Ok(Fetched {
        manifest,
        body,
        signature: sig.trim().to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Cache {
    /// Unix seconds of the last successful check.
    #[serde(default)]
    pub checked_at: u64,
    /// Parsed from [`Cache::manifest`] when the cache is loaded, and only
    /// when [`Cache::signature`] verifies: never read from the file.
    #[serde(skip)]
    pub latest: Option<Manifest>,
    /// The last verified `latest.json`, byte for byte.
    #[serde(default)]
    pub manifest: Option<String>,
    /// Its signature (base64).
    #[serde(default)]
    pub signature: Option<String>,
    /// "Later" was chosen for this version: no chip until a newer one.
    #[serde(default)]
    pub dismissed: Option<String>,
}

impl Cache {
    /// Record a verified check's manifest.
    pub fn store(&mut self, f: &Fetched) {
        self.latest = Some(f.manifest.clone());
        self.manifest = Some(f.body.clone());
        self.signature = Some(f.signature.clone());
    }

    /// A check is due at `now` (Unix seconds).
    pub fn due(&self, now: u64) -> bool {
        now.saturating_sub(self.checked_at) >= CHECK_EVERY.as_secs() || self.checked_at > now
    }

    /// Seconds until the next check is due.
    pub fn due_in(&self, now: u64) -> u64 {
        if self.due(now) {
            0
        } else {
            CHECK_EVERY.as_secs() - now.saturating_sub(self.checked_at)
        }
    }
}

fn data_dir() -> Option<PathBuf> {
    crate::shell_integration::data_dir()
}

pub fn cache_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("update.json"))
}

/// The cache at `path`. The file is writable by anything running as the
/// user, so it is trusted no more than the network: the manifest is
/// re-verified against the keys trusted for the current manifest URL and
/// re-validated; one that fails (or a cache without a signed manifest, as
/// older versions wrote) is dropped, so the next check refetches. A
/// dismissed version that is not a release version is dropped too.
pub fn load_cache_from(path: &Path) -> Cache {
    load_cache_with(path, &trusted_keys(&manifest_url()), url_overridden())
}

/// [`load_cache_from`] with explicit keys and test override.
pub fn load_cache_with(path: &Path, keys: &[[u8; 32]], allow_file: bool) -> Cache {
    let mut c: Cache = std::fs::read(path)
        .ok()
        // the manifest is stored as a JSON string: escapes can grow it
        .filter(|b| b.len() <= MAX_MANIFEST * 8)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let verified = match (&c.manifest, &c.signature) {
        (Some(body), Some(sig)) => open_signed(body.as_bytes(), sig, keys, allow_file).ok(),
        _ => None,
    };
    match verified {
        Some(f) => c.latest = Some(f.manifest),
        None => {
            c.latest = None;
            c.manifest = None;
            c.signature = None;
            c.checked_at = 0;
        }
    }
    if c.dismissed
        .as_deref()
        .is_some_and(|d| !valid_release_version(d))
    {
        c.dismissed = None;
    }
    c
}

pub fn load_cache() -> Cache {
    cache_path()
        .map(|p| load_cache_from(&p))
        .unwrap_or_default()
}

/// Atomic write (tmp + rename).
pub fn save_cache_to(path: &Path, cache: &Cache) -> std::io::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(cache)?)
}

pub fn save_cache(cache: &Cache) {
    if let Some(p) = cache_path() {
        let _ = save_cache_to(&p, cache);
    }
}

/// Serializes this process's read-modify-write of the cache (the periodic
/// and a manual check can finish together). Other windows are processes of
/// their own: each change re-reads the file right before writing it.
static CACHE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Load, change and save the cache.
pub fn update_cache(f: impl FnOnce(&mut Cache)) {
    let _held = CACHE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = load_cache();
    f(&mut c);
    save_cache(&c);
}

/// Atomic write (unique tmp + rename), readable by the user only.
fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{}-{n}", std::process::id()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(body));
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

// ── network (system curl) ──

/// A system tool: by absolute path on macOS (a `curl` or `hdiutil` earlier
/// on the inherited PATH must not run), from PATH elsewhere.
fn tool(name: &str) -> std::process::Command {
    if cfg!(target_os = "macos") {
        std::process::Command::new(format!("/usr/bin/{name}"))
    } else {
        std::process::Command::new(name)
    }
}

/// `curl` restricted to https (and file:// for test overrides), failing on
/// HTTP errors. `-q` (first) ignores the user's `~/.curlrc`.
/// Why `curl` did not start; a missing binary says so plainly (minimal
/// Linux installs ship without it).
fn curl_spawn_error(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        "curl is not installed".into()
    } else {
        format!("curl: {e}")
    }
}

fn curl(allow_file: bool) -> std::process::Command {
    let mut c = tool("curl");
    let protos = if allow_file { "=https,file" } else { "=https" };
    c.args(["-q", "-fsSL", "--proto", protos, "--proto-redir", "=https"])
        .stdin(std::process::Stdio::null());
    c
}

/// Fetch the manifest at `url` and its signature (`url` + [`SIG_SUFFIX`]),
/// verify the signature over the manifest's exact bytes with the keys
/// trusted for `url`, then validate it (blocking; run off the UI thread).
pub fn fetch(url: &str) -> Result<Fetched, String> {
    fetch_with(url, &trusted_keys(url))
}

/// [`fetch`] with explicit keys.
pub fn fetch_with(url: &str, keys: &[[u8; 32]]) -> Result<Fetched, String> {
    let allow_file = url.starts_with("file://");
    if !url_allowed(url, allow_file) {
        return Err(format!("unsupported URL {url}"));
    }
    let body = fetch_bytes(url, allow_file, MAX_MANIFEST, "manifest too large")?;
    let sig = fetch_bytes(
        &format!("{url}{SIG_SUFFIX}"),
        allow_file,
        MAX_SIG,
        SIG_INVALID,
    )
    .map_err(|e| {
        if e == SIG_INVALID {
            e
        } else {
            log::info!("update check: {url}{SIG_SUFFIX}: {e}");
            SIG_MISSING.to_string()
        }
    })?;
    let sig = std::str::from_utf8(&sig).map_err(|_| SIG_INVALID.to_string())?;
    open_signed(&body, sig, keys, allow_file)
}

/// The body at `url`, at most `max` bytes (`too_large` otherwise).
fn fetch_bytes(
    url: &str,
    allow_file: bool,
    max: usize,
    too_large: &str,
) -> Result<Vec<u8>, String> {
    let mut child = curl(allow_file)
        .args(["--max-time", "15", url])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(curl_spawn_error)?;
    let mut body = Vec::new();
    let read = child
        .stdout
        .take()
        .map(|o| o.take(max as u64 + 1).read_to_end(&mut body));
    if body.len() > max {
        let _ = child.kill();
        let _ = child.wait();
        return Err(too_large.into());
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if let Some(Err(e)) = read {
        return Err(format!("curl: {e}"));
    }
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim().trim_start_matches("curl: ");
        return Err(if err.is_empty() {
            format!("curl exited with {}", out.status)
        } else {
            err.chars().take(160).collect()
        });
    }
    Ok(body)
}

/// Lowercase hex SHA-256 of `path` (`shasum -a 256` on macOS, `sha256sum`
/// elsewhere).
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let tools: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("shasum", &["-a", "256"])]
    } else {
        &[("sha256sum", &[]), ("shasum", &["-a", "256"])]
    };
    let mut last = String::from("no sha256 tool");
    for &(name, args) in tools {
        let tool = name;
        match self::tool(name)
            .args(args)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .output()
        {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let digest = text.split_whitespace().next().unwrap_or("");
                if digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Ok(digest.to_ascii_lowercase());
                }
                last = format!("{tool}: unexpected output");
            }
            Ok(out) => last = format!("{tool} exited with {}", out.status),
            Err(e) => last = format!("{tool}: {e}"),
        }
    }
    Err(last)
}

/// Download `asset` to `dest` and verify its digest; a mismatch deletes the
/// file.
pub fn download(asset: &Asset, dest: &Path) -> Result<(), String> {
    download_with(asset, dest, url_overridden(), MAX_ASSET)
}

/// Where an asset is downloaded inside `updates/`: a fixed name (never
/// derived from the manifest).
pub fn download_path(dir: &Path, asset: &Asset) -> PathBuf {
    dir.join(if asset.kind == "dmg" {
        "download.dmg"
    } else {
        "download.AppImage"
    })
}

/// Where an update downloaded ahead of time waits in `updates/` (a fixed
/// name; `sweep` leaves it): the install takes it when its SHA-256 still
/// matches the manifest, so a stale or partial file is never installed.
pub fn ready_path(dir: &Path, asset: &Asset) -> PathBuf {
    dir.join(if asset.kind == "dmg" {
        "ready.dmg"
    } else {
        "ready.AppImage"
    })
}

/// Delete the downloads waiting in `dir` (this version is the newest, so
/// none is needed; an install that wanted one downloads it again).
pub fn drop_ready(dir: &Path) {
    for name in ["ready.dmg", "ready.AppImage"] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// The ready download for `asset`, when one is there and intact.
pub fn ready_for(dir: &Path, asset: &Asset) -> Option<PathBuf> {
    let path = ready_path(dir, asset);
    sha256_file(&path)
        .ok()
        .filter(|d| d.eq_ignore_ascii_case(asset.sha256.trim()))
        .map(|_| path)
}

/// Download `asset` ahead of the install (Rift notifies once it is here):
/// into a file of this process, verified, then renamed to [`ready_path`],
/// so two windows fetching at once never leave a half-written file there.
/// Takes the ready file as it is when it already matches.
pub fn prefetch(dir: &Path, asset: &Asset) -> Result<PathBuf, String> {
    prefetch_with(dir, asset, url_overridden(), MAX_ASSET)
}

fn prefetch_with(dir: &Path, asset: &Asset, allow_file: bool, max: u64) -> Result<PathBuf, String> {
    if let Some(ready) = ready_for(dir, asset) {
        return Ok(ready);
    }
    // what a fetch that died left (a live one finishes well inside an hour)
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|a| a > Duration::from_secs(3600));
            if old && e.file_name().to_string_lossy().starts_with("prefetch-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let part = dir.join(format!("prefetch-{}.{}", std::process::id(), asset.kind));
    download_with(asset, &part, allow_file, max)?;
    let ready = ready_path(dir, asset);
    std::fs::rename(&part, &ready).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("{}: {e}", ready.display())
    })?;
    Ok(ready)
}

fn download_with(asset: &Asset, dest: &Path, allow_file: bool, max: u64) -> Result<(), String> {
    if !asset_url_allowed(&asset.url, allow_file) {
        return Err(format!("unsupported URL {}", asset.url));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let _ = std::fs::remove_file(dest);
    let out = curl(allow_file)
        .args([
            "--max-time",
            "900",
            "--max-filesize",
            // curl's own limit stays MAX_ASSET (`max` lowers only the check
            // below, which tests reach through a file:// URL with a size)
            &MAX_ASSET.to_string(),
        ])
        .arg("-o")
        .arg(dest)
        .arg(&asset.url)
        .output()
        .map_err(curl_spawn_error)?;
    if !out.status.success() {
        let _ = std::fs::remove_file(dest);
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "download failed: {}",
            err.trim().trim_start_matches("curl: ")
        ));
    }
    // --max-filesize trusts Content-Length; a body without one is checked here
    let size = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(u64::MAX);
    if size > max {
        let _ = std::fs::remove_file(dest);
        return Err("download is too large; nothing was changed".into());
    }
    let digest = sha256_file(dest)?;
    if !digest.eq_ignore_ascii_case(asset.sha256.trim()) {
        let _ = std::fs::remove_file(dest);
        return Err("download is corrupt (SHA-256 mismatch); nothing was changed".into());
    }
    Ok(())
}

// ── install ──

/// Where this copy of Rift can be replaced.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallTarget {
    /// `…/Rift.app` (its parent directory is writable).
    MacApp(PathBuf),
    /// The AppImage file (`$APPIMAGE`; its directory is writable).
    AppImage(PathBuf),
    /// Updating in place is not possible; the reason, for the dialog.
    Unsupported(String),
}

/// Whether a directory can take the new copy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DirAccess {
    Writable,
    /// Permissions forbid it.
    Denied,
    /// A read-only volume (a mounted disk image).
    ReadOnlyVolume,
}

/// Pure detection: `exe` is the running binary, `appimage` and `appdir`
/// the AppImage runtime's `$APPIMAGE` and `$APPDIR` (only trusted when
/// `exe` runs from inside `appdir`: the variables are inherited by anything
/// started from a Rift shell), `access` how a directory can be written.
pub fn install_target_for(
    os: &str,
    exe: &Path,
    appimage: Option<&Path>,
    appdir: Option<&Path>,
    access: impl Fn(&Path) -> DirAccess,
) -> InstallTarget {
    let in_dir = |dir: &Path, ok: InstallTarget| match access(dir) {
        DirAccess::Writable => ok,
        DirAccess::Denied => {
            InstallTarget::Unsupported(format!("{} is not writable", dir.display()))
        }
        DirAccess::ReadOnlyVolume => InstallTarget::Unsupported(MOVE_TO_APPLICATIONS.into()),
    };
    match os {
        "macos" => {
            // …/X.app/Contents/MacOS/rift-terminal
            let macos = exe.parent();
            let contents = macos.and_then(Path::parent);
            let app = contents.and_then(Path::parent);
            let is_bundle = macos.and_then(Path::file_name) == Some("MacOS".as_ref())
                && contents.and_then(Path::file_name) == Some("Contents".as_ref())
                && app.is_some_and(|a| a.extension().is_some_and(|e| e == "app"));
            match app {
                // Gatekeeper runs a quarantined app from a random read-only
                // copy: replacing that copy would not update the real one
                Some(app) if is_bundle && app.to_string_lossy().contains("/AppTranslocation/") => {
                    InstallTarget::Unsupported(MOVE_TO_APPLICATIONS.into())
                }
                Some(app) if is_bundle => match app.parent() {
                    Some(dir) => in_dir(dir, InstallTarget::MacApp(app.to_path_buf())),
                    None => InstallTarget::Unsupported("no app folder".into()),
                },
                _ => InstallTarget::Unsupported("not running from Rift.app".into()),
            }
        }
        "linux" => match (appimage, appdir) {
            (Some(img), Some(dir)) if exe.starts_with(dir) && img.is_absolute() => {
                match img.parent() {
                    Some(parent) => in_dir(parent, InstallTarget::AppImage(img.to_path_buf())),
                    None => InstallTarget::Unsupported("no AppImage folder".into()),
                }
            }
            _ => InstallTarget::Unsupported(
                "installed from a package or archive; update it the same way".into(),
            ),
        },
        other => InstallTarget::Unsupported(format!("updates are not supported on {other}")),
    }
}

/// `access(dir, W_OK)`, telling a read-only volume (`EROFS`) apart.
pub fn dir_access(dir: &Path) -> DirAccess {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return DirAccess::Denied;
    };
    if !dir.is_dir() {
        return DirAccess::Denied;
    }
    if unsafe { libc_access(c.as_ptr(), 2) } == 0 {
        return DirAccess::Writable;
    }
    // EROFS is 30 on macOS and Linux
    if std::io::Error::last_os_error().raw_os_error() == Some(30) {
        DirAccess::ReadOnlyVolume
    } else {
        DirAccess::Denied
    }
}

unsafe extern "C" {
    #[link_name = "access"]
    fn libc_access(path: *const std::ffi::c_char, mode: i32) -> i32;
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
    #[cfg(target_os = "macos")]
    fn renamex_np(from: *const std::ffi::c_char, to: *const std::ffi::c_char, flags: u32) -> i32;
}

/// This process's install target.
pub fn install_target() -> InstallTarget {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return InstallTarget::Unsupported(format!("no executable path: {e}")),
    };
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from);
    install_target_for(
        std::env::consts::OS,
        &exe,
        appimage.as_deref(),
        appdir.as_deref(),
        dir_access,
    )
}

/// The app path of a target (bundle or AppImage).
pub fn path_of(target: &InstallTarget) -> Option<&Path> {
    match target {
        InstallTarget::MacApp(p) | InstallTarget::AppImage(p) => Some(p),
        InstallTarget::Unsupported(_) => None,
    }
}

/// The marker the new version clears once it has run for [`CONFIRM_AFTER`];
/// the relaunch helper rolls back when the new process dies before that.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub from: String,
    pub to: String,
    /// The previous app (`.Rift-old.app` / `<AppImage>.old`), deleted after
    /// a successful start.
    pub old_path: PathBuf,
    /// The installed app (bundle or AppImage).
    pub new_path: PathBuf,
    /// This update's secret: other windows accept `prepare-restart` only
    /// with it (the marker is readable by the user only).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
}

/// How long the new version runs after its first frame before the update
/// counts as done (marker and old copy removed, other windows reopened).
pub const CONFIRM_AFTER: Duration = Duration::from_secs(10);
/// How old a marker may be for its token to be accepted.
pub const TOKEN_TTL: Duration = Duration::from_secs(60);

pub fn updates_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("updates"))
}

pub fn pending_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("update-pending.json"))
}

/// Written by the helper when it rolled back.
pub fn failed_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("update-failed.json"))
}

pub fn read_pending(path: &Path) -> Option<Pending> {
    let body = std::fs::read(path)
        .ok()
        .filter(|b| b.len() <= MAX_MANIFEST)?;
    serde_json::from_slice(&body).ok()
}

/// Atomic, readable by the user only (it holds the token).
pub fn write_pending(path: &Path, p: &Pending) -> std::io::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(p)?)
}

/// A fresh random token (16 bytes of `/dev/urandom`, hex).
pub fn new_token() -> std::io::Result<String> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// A `prepare-restart` with `token` belongs to an update in flight: the
/// marker (written at `written`) carries that token and is younger than
/// [`TOKEN_TTL`] at `now`.
pub fn token_valid(
    p: &Pending,
    token: &str,
    written: std::time::SystemTime,
    now: std::time::SystemTime,
) -> bool {
    !token.is_empty()
        && p.token == token
        && now.duration_since(written).is_ok_and(|age| age < TOKEN_TTL)
}

/// The marker's paths are this install's app and its sibling backup (as
/// [`old_path_of`] names it): only then are they deleted or moved.
pub fn pending_matches_target(p: &Pending, target: &InstallTarget) -> bool {
    path_of(target) == Some(p.new_path.as_path())
        && old_path_of(target).as_deref() == Some(p.old_path.as_path())
}

/// The executable inside an install target.
pub fn executable_of(target: &InstallTarget) -> Option<PathBuf> {
    match target {
        InstallTarget::MacApp(app) => Some(app.join("Contents/MacOS/rift-terminal")),
        InstallTarget::AppImage(p) => Some(p.clone()),
        InstallTarget::Unsupported(_) => None,
    }
}

/// Where the replaced app is kept until the new one starts.
pub fn old_path_of(target: &InstallTarget) -> Option<PathBuf> {
    match target {
        InstallTarget::MacApp(app) => {
            Some(app.with_file_name(format!(".{}-old.app", app.file_stem()?.to_string_lossy())))
        }
        InstallTarget::AppImage(p) => {
            Some(p.with_file_name(format!("{}.old", p.file_name()?.to_string_lossy())))
        }
        InstallTarget::Unsupported(_) => None,
    }
}

/// Where the new app is staged beside the current one (same filesystem).
fn staged_path_of(target: &InstallTarget) -> Option<PathBuf> {
    match target {
        InstallTarget::MacApp(app) => Some(app.with_file_name(format!(
            ".{}-update.app",
            app.file_stem()?.to_string_lossy()
        ))),
        InstallTarget::AppImage(p) => {
            Some(p.with_file_name(format!(".{}.update", p.file_name()?.to_string_lossy())))
        }
        InstallTarget::Unsupported(_) => None,
    }
}

// ── processes ──

/// `pid` exists (`kill(pid, 0)`; a process of another user counts).
pub fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    pid > 0
        && (unsafe { libc_kill(pid, 0) } == 0
            // EPERM is 1 on macOS and Linux
            || std::io::Error::last_os_error().raw_os_error() == Some(1))
}

/// Run `cmd` to completion within `limit` (killed after it), returning its
/// stdout, or its stderr as the error.
fn run_for(cmd: &mut std::process::Command, limit: Duration) -> Result<String, String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let output = crate::process::output(cmd, None, limit, 1 << 20).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            format!("{name}: timed out after {} s", limit.as_secs())
        } else {
            format!("{name}: {e}")
        }
    })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The version a staged binary reports (`--version`).
fn staged_version(exe: &Path) -> Result<String, String> {
    let out = run_for(
        std::process::Command::new(exe).arg("--version"),
        Duration::from_secs(15),
    )?;
    Ok(out
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .to_string())
}

// ── one install at a time ──

/// `updates/install.lock`, held from the download until this process
/// exits or the install fails: two windows must never stage, swap or
/// delete the backup at the same time.
#[derive(Debug)]
pub struct InstallLock {
    path: PathBuf,
    // Never unlink this file: all contenders must lock the same inode,
    // including while a stale PID file is replaced or removed on drop.
    _guard: InstallGuard,
}

#[derive(Debug)]
struct InstallGuard(std::fs::File);

impl Drop for InstallGuard {
    fn drop(&mut self) {
        // A concurrently forked child can inherit this open description
        // until exec. Explicit unlock releases it without waiting for that fd.
        let _ = self.0.unlock();
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, PartialEq)]
pub enum LockError {
    /// Another running process holds it.
    Held(u32),
    Io(String),
}

/// Take the install lock in `dir`; a lock whose process is gone is stale
/// and taken over.
pub fn try_lock(dir: &Path) -> Result<InstallLock, LockError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(dir).map_err(|e| LockError::Io(format!("{}: {e}", dir.display())))?;
    let path = dir.join("install.lock");
    let guard = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join("install.guard"))
        .map_err(|e| LockError::Io(format!("{}: {e}", dir.display())))?;
    match guard.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            let pid = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            return Err(LockError::Held(pid));
        }
        Err(std::fs::TryLockError::Error(e)) => return Err(LockError::Io(e.to_string())),
    }
    let guard = InstallGuard(guard);
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut f) => {
                if let Err(e) = write!(f, "{}", std::process::id()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(LockError::Io(e.to_string()));
                }
                return Ok(InstallLock {
                    path,
                    _guard: guard,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let holder = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                let fresh = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t.elapsed().is_ok_and(|a| a < Duration::from_secs(5)));
                match holder {
                    Some(pid) if pid_alive(pid) => {
                        return Err(LockError::Held(pid));
                    }
                    // being written right now
                    None if fresh => return Err(LockError::Held(0)),
                    _ => {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
            Err(e) => return Err(LockError::Io(format!("{}: {e}", path.display()))),
        }
    }
    Err(LockError::Io("the install lock is busy".into()))
}

/// Remove what an interrupted install left in `dir` (downloads, disk-image
/// mount points, detached first) and the staged copy beside `target`'s app.
/// Call with the lock held.
pub fn sweep(dir: &Path, target: &InstallTarget) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("download.") {
                continue;
            }
            if p.is_dir() && name.contains(".mnt") {
                if cfg!(target_os = "macos") && std::fs::remove_dir(&p).is_err() {
                    let _ = run_for(
                        tool("hdiutil").arg("detach").arg(&p).arg("-force"),
                        Duration::from_secs(60),
                    );
                }
                let _ = std::fs::remove_dir(&p);
            } else if p.is_file() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    if let Some(staged) = staged_path_of(target) {
        remove_any(&staged);
    }
}

/// Install the verified `file` (a DMG or AppImage) as `version` over
/// `target`: stage the new app next to the old one, check it reports
/// `version`, then swap them. On any error the old app is left in place.
/// Returns the marker to write.
pub fn install(
    target: &InstallTarget,
    file: &Path,
    version: &str,
    from: &str,
) -> Result<Pending, String> {
    let old = old_path_of(target).ok_or("cannot update this install")?;
    let path = match target {
        InstallTarget::MacApp(p) | InstallTarget::AppImage(p) => p.clone(),
        InstallTarget::Unsupported(why) => return Err(why.clone()),
    };
    let staged = staged_path_of(target).ok_or("cannot update this install")?;
    match target {
        InstallTarget::MacApp(_) => stage_dmg(&staged, file)?,
        InstallTarget::AppImage(_) => stage_appimage(&staged, file)?,
        InstallTarget::Unsupported(_) => unreachable!(),
    }
    let staged_exe = match target {
        InstallTarget::MacApp(_) => staged.join("Contents/MacOS/rift-terminal"),
        _ => staged.clone(),
    };
    let reported = staged_version(&staged_exe).inspect_err(|_| remove_any(&staged))?;
    if !same_version(&reported, version) {
        remove_any(&staged);
        return Err(format!(
            "the download is version {reported}, expected {version}; nothing was changed"
        ));
    }
    swap(&path, &staged, &old)?;
    Ok(Pending {
        from: from.into(),
        to: version.into(),
        old_path: old,
        new_path: path,
        token: String::new(),
    })
}

fn remove_any(p: &Path) {
    if p.is_dir() && !p.is_symlink() {
        let _ = std::fs::remove_dir_all(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(target_os = "macos")]
fn c_path(p: &Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(std::io::Error::other)
}

/// Atomically exchange `a` and `b` (`renamex_np(RENAME_SWAP)`): neither
/// path is ever missing.
#[cfg(target_os = "macos")]
fn exchange(a: &Path, b: &Path) -> std::io::Result<()> {
    const RENAME_SWAP: u32 = 0x2;
    let (ca, cb) = (c_path(a)?, c_path(b)?);
    if unsafe { renamex_np(ca.as_ptr(), cb.as_ptr(), RENAME_SWAP) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn exchange(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Put `staged` in place of `current`, keeping the replaced app at `old`.
/// The app path is never missing: an atomic exchange (macOS), or a hard
/// link to the old file then an atomic rename over it (AppImage); two
/// renames, undone on failure, only where neither works.
fn swap(current: &Path, staged: &Path, old: &Path) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("{}: {e}", current.display());
    remove_any(old);
    if !staged.exists() {
        return Err(format!("{}: the staged copy is missing", staged.display()));
    }
    if exchange(staged, current).is_ok() {
        // `staged` now holds the previous app
        if let Err(e) = std::fs::rename(staged, old) {
            let _ = exchange(staged, current);
            remove_any(staged);
            return Err(fail(e));
        }
        return Ok(());
    }
    if current.is_file() && std::fs::hard_link(current, old).is_ok() {
        if let Err(e) = std::fs::rename(staged, current) {
            remove_any(old);
            remove_any(staged);
            return Err(fail(e));
        }
        return Ok(());
    }
    std::fs::rename(current, old).map_err(|e| {
        remove_any(staged);
        fail(e)
    })?;
    if let Err(e) = std::fs::rename(staged, current) {
        let _ = std::fs::rename(old, current);
        remove_any(staged);
        return Err(fail(e));
    }
    Ok(())
}

/// Undo an install that will not be started: the old app goes back to
/// `new_path` (atomically where possible) and the new one is deleted.
pub fn restore_old(p: &Pending) -> Result<(), String> {
    let (cur, old) = (&p.new_path, &p.old_path);
    if !old.exists() {
        return Err(format!("{}: missing", old.display()));
    }
    if exchange(old, cur).is_ok() {
        remove_any(old);
        return Ok(());
    }
    if cur.is_file() && old.is_file() {
        return std::fs::rename(old, cur).map_err(|e| format!("{}: {e}", cur.display()));
    }
    let failed = cur.with_file_name(format!(
        "{}.failed",
        cur.file_name().unwrap_or_default().to_string_lossy()
    ));
    remove_any(&failed);
    std::fs::rename(cur, &failed).map_err(|e| format!("{}: {e}", cur.display()))?;
    if let Err(e) = std::fs::rename(old, cur) {
        let _ = std::fs::rename(&failed, cur);
        return Err(format!("{}: {e}", cur.display()));
    }
    remove_any(&failed);
    Ok(())
}

/// Mount the DMG read-only, copy its `.app` to `staged`, unmount.
fn stage_dmg(staged: &Path, dmg: &Path) -> Result<(), String> {
    remove_any(staged);
    let mnt = dmg.with_extension(format!("mnt{}", std::process::id()));
    let _ = std::fs::create_dir_all(&mnt);
    if let Err(e) = run_for(
        tool("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mnt)
            .arg(dmg),
        Duration::from_secs(120),
    ) {
        // a timed-out attach may still mount: detach before removing
        let _ = run_for(
            tool("hdiutil").arg("detach").arg(&mnt).arg("-force"),
            Duration::from_secs(60),
        );
        let _ = std::fs::remove_dir(&mnt);
        return Err(e);
    }
    let result = (|| {
        let inner = std::fs::read_dir(&mnt)
            .map_err(|e| format!("{}: {e}", mnt.display()))?
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.extension().is_some_and(|e| e == "app")
                    && !p
                        .symlink_metadata()
                        .is_ok_and(|m| m.file_type().is_symlink())
            })
            .ok_or("the disk image holds no app")?;
        run_for(
            tool("ditto").arg(&inner).arg(staged),
            Duration::from_secs(300),
        )?;
        Ok(())
    })();
    if run_for(
        tool("hdiutil").arg("detach").arg(&mnt).arg("-quiet"),
        Duration::from_secs(60),
    )
    .is_err()
    {
        let _ = run_for(
            tool("hdiutil").arg("detach").arg(&mnt).arg("-force"),
            Duration::from_secs(60),
        );
    }
    let _ = std::fs::remove_dir(&mnt);
    if result.is_err() {
        remove_any(staged);
    }
    result
}

/// Copy the AppImage to `staged` (beside the current one: same filesystem
/// for the rename), executable.
fn stage_appimage(staged: &Path, file: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    remove_any(staged);
    std::fs::copy(file, staged).map_err(|e| format!("{}: {e}", staged.display()))?;
    std::fs::set_permissions(staged, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("{}: {e}", staged.display()))?;
    Ok(())
}

/// The relaunch helper (`/bin/sh -c HELPER sh pid target old exe marker
/// failed log args…`): wait (at most 60 s) for `pid` to exit, start `exe
/// args…`, and roll back only when that process DIES while `marker` is still
/// there (the new version clears it after running for [`CONFIRM_AFTER`]; the
/// helper moves the marker away before rolling back, so a version that
/// cleared it and quit at once is kept):
/// `old` goes back over `target` and starts with the same arguments. A new
/// process still alive but unconfirmed after 120 s is left running. A zombie
/// counts as exited. A process's state comes from `/proc` where it exists
/// (Linux: `ps` may be missing or busybox's, which would read as "exited"
/// and roll back every update), else from `ps`. `log` is emptied first once
/// it passes 1 MB.
pub const HELPER: &str = r#"pid=$1 target=$2 old=$3 exe=$4 marker=$5 failed=$6 log=$7
shift 7
if [ -f "$log" ] && [ "$(wc -c <"$log")" -gt 1048576 ]; then : >"$log"; fi
gone() {
    if [ -r /proc/self/stat ]; then
        s=$(sed 's/.*) //' "/proc/$1/stat" 2>/dev/null)
    else
        s=$(ps -o stat= -p "$1" 2>/dev/null)
    fi
    case "$s" in
        ''|Z*) return 0 ;;
    esac
    return 1
}
i=0
while ! gone "$pid"; do
    i=$((i + 1))
    if [ "$i" -gt 600 ]; then
        echo "rift update: process $pid did not exit; not relaunching" >>"$log"
        exit 1
    fi
    sleep 0.1
done
"$exe" "$@" </dev/null >>"$log" 2>&1 &
child=$!
i=0
while [ -e "$marker" ]; do
    if gone "$child"; then
        # taking the marker decides: a version that cleared it and quit
        # between the two checks confirmed, and is kept
        mv -f "$marker" "$failed" 2>/dev/null || exit 0
        if [ -e "$old" ]; then
            rm -rf "$target.failed"
            mv "$target" "$target.failed" 2>/dev/null
            if mv "$old" "$target"; then
                rm -rf "$target.failed"
            elif [ -e "$target.failed" ]; then
                mv "$target.failed" "$target"
            fi
        fi
        echo "rift update: the new version exited before confirming; rolled back" >>"$log"
        exec "$exe" "$@" </dev/null >>"$log" 2>&1
    fi
    i=$((i + 1))
    [ "$i" -gt 1200 ] && exit 0
    sleep 0.1
done
exit 0
"#;

/// Arguments for the relaunched process: the original ones without
/// `--script FILE` and LaunchServices' `-psn_…`; `relaunch_script` (a test
/// hook) is appended as a new `--script`, and without it `--headless`
/// (which needs a script) goes too.
pub fn relaunch_args(original: &[String], relaunch_script: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = original.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--script" => {
                it.next();
            }
            "--headless" if relaunch_script.is_none() => {}
            psn if psn.starts_with("-psn_") => {}
            _ => out.push(a.clone()),
        }
    }
    if let Some(s) = relaunch_script {
        out.push("--script".into());
        out.push(s.into());
    }
    out
}

/// Start the detached helper (its own process group: it outlives this
/// process). `env` is added to the inherited environment, `unset` removed.
#[allow(clippy::too_many_arguments)]
pub fn spawn_helper(
    pending: &Pending,
    exe: &Path,
    marker: &Path,
    failed: &Path,
    log: &Path,
    args: &[String],
    env: &[(String, String)],
    unset: &[&str],
) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(HELPER)
        .arg("sh")
        .arg(std::process::id().to_string())
        .arg(&pending.new_path)
        .arg(&pending.old_path)
        .arg(exe)
        .arg(marker)
        .arg(failed)
        .arg(log)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    for k in unset {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rift-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn versions_compare_numerically_with_prereleases_first() {
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.2") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.9"));
        assert!(v("1.0.0-beta") > v("1.0.0-alpha"));
        assert!(v("1.0.0-alpha.beta") > v("1.0.0-alpha.1"));
        assert_eq!(v("v1.2"), v("1.2.0"));
        assert_eq!(v("1.2.3+build.5"), v("1.2.3"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("99.0.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("0.2.0-rc.1", "0.2.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        for bad in [
            "",
            "v",
            "1.2.3.4",
            "1..2",
            "1.x",
            "1.2.3-",
            "1.2.3-a b",
            "-1.0",
        ] {
            assert!(Version::parse(bad).is_none(), "{bad:?}");
        }
        // the marker's `to` and the running version compare as versions
        assert!(same_version("v0.2.0", "0.2.0"));
        assert!(!same_version("0.2.0", "0.2.1"));
        assert!(!same_version("x", "x"));
    }

    #[test]
    fn release_versions_are_strict() {
        for good in ["0.2.0", "v0.2.0", "10.20.30", "1.0.0-rc.1", "1.0.0-beta2"] {
            assert!(valid_release_version(good), "{good:?}");
        }
        for bad in [
            "0.2.0+/../../x",
            "0.2.0+build",
            "0.2",
            "0.2.0.1",
            "0.2.0-",
            "0.2.0-a/b",
            "0.2.0-../x",
            "0.2.0 ",
            "",
            "1234567890.0.0",
        ] {
            assert!(!valid_release_version(bad), "{bad:?}");
        }
    }

    const GOOD: &str = r#"{"version":"0.2.0","date":"2026-11-01","protocol":1,
        "notes":"Faster.\n\nMore.","url":"https://github.com/botjaeger/rift-releases/releases/tag/v0.2.0",
        "assets":{"macos-aarch64":{"url":"https://github.com/botjaeger/rift-releases/releases/download/v0.2.0/Rift-0.2.0-arm64.dmg","sha256":"ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789","kind":"dmg"}}}"#;
    const DL: &str = "https://github.com/botjaeger/rift-releases/releases/download/v0.2.0";

    #[test]
    fn manifests_are_validated() {
        let m = parse_manifest(GOOD.as_bytes(), false).unwrap();
        assert_eq!(m.version, "0.2.0");
        assert_eq!(m.protocol, Some(1));
        assert_eq!(asset_for(&m, "macos-aarch64").unwrap().kind, "dmg");
        assert!(asset_for(&m, "linux-x86_64").is_none());
        assert_eq!(summary(&m.notes, 80), "Faster.");
        assert_eq!(summary("a b c d e f", 5), "a b …");
        // unknown fields are fine (newer manifests)
        let extra = GOOD.replacen('{', "{\"future\":1,", 1);
        assert!(parse_manifest(extra.as_bytes(), false).is_ok());
        for (what, body) in [
            ("not json", "nope".to_string()),
            ("no version", r#"{"date":"x"}"#.to_string()),
            ("bad version", GOOD.replace("\"0.2.0\"", "\"zero\"")),
            (
                "version with a path",
                GOOD.replace("\"0.2.0\"", "\"0.2.0+/../../x\""),
            ),
            ("short version", GOOD.replace("\"0.2.0\"", "\"0.2\"")),
            ("http", GOOD.replace(DL, "http://github.com/x")),
            ("another host", GOOD.replace(DL, "https://example.com/x")),
            (
                "another repository",
                GOOD.replace(
                    DL,
                    "https://github.com/evil/rift-releases/releases/download/v1",
                ),
            ),
            (
                "dot segments",
                GOOD.replace(DL, &format!("{DL}/../../../../evil")),
            ),
            ("query", GOOD.replace("arm64.dmg", "arm64.dmg?x=1")),
            ("escape", GOOD.replace("arm64.dmg", "%2e%2e")),
            ("file", GOOD.replace(DL, "file:///tmp")),
            (
                "bad kind",
                GOOD.replace("\"kind\":\"dmg\"", "\"kind\":\"exe\""),
            ),
            ("short sha", GOOD.replace("ABCDEF", "")),
            ("nonhex sha", GOOD.replace("ABCDEF", "ABCDEZ")),
            ("too large", format!("{GOOD}{}", " ".repeat(MAX_MANIFEST))),
        ] {
            assert!(parse_manifest(body.as_bytes(), false).is_err(), "{what}");
        }
        // a test override may point assets at files
        let file = GOOD.replace(DL, "file:///tmp");
        assert!(parse_manifest(file.as_bytes(), true).is_ok());
    }

    #[test]
    fn evaluation_and_protocol() {
        let m = parse_manifest(GOOD.as_bytes(), false).unwrap();
        assert!(matches!(evaluate(m.clone(), "0.1.0"), Status::Available(_)));
        assert_eq!(evaluate(m.clone(), "0.2.0"), Status::UpToDate);
        assert_eq!(evaluate(m.clone(), "0.3.0"), Status::UpToDate);
        assert!(protocol_compatible(&m, 1));
        assert!(!protocol_compatible(&m, 2));
        let old = Manifest {
            protocol: None,
            ..m.clone()
        };
        assert!(!protocol_compatible(&old, 1));
    }

    #[test]
    fn asset_keys_name_os_and_arch() {
        assert_eq!(asset_key("macos", "aarch64"), "macos-aarch64");
        assert_eq!(asset_key("macos", "x86_64"), "macos-x86_64");
        assert_eq!(asset_key("linux", "x86_64"), "linux-x86_64");
        assert_eq!(
            platform_key(),
            format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
        );
    }

    /// A throwaway key pair for tests only (never a release key).
    const TEST_SEED: &str = "a662e55aa87b9685d68b2d2f1f6f19e93459b7a4d6d1593014731cbb90c9737c";
    const TEST_PUBLIC: &str = "3ew5tzoYl65wxNCdUnBk94p1jI+QRrnMjNxgw9DPRTY=";

    fn test_pair() -> ed25519_compact::KeyPair {
        let seed: Vec<u8> = (0..TEST_SEED.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&TEST_SEED[i..i + 2], 16).unwrap())
            .collect();
        ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::from_slice(&seed).unwrap())
    }

    fn b64(data: &[u8]) -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= chunk.len() {
                    A[(n >> (18 - 6 * i) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    }

    /// The test key's signature of `body`, as `latest.json.sig` holds it.
    fn sign(body: &[u8]) -> String {
        b64(&test_pair().sk.sign(body, None)[..])
    }

    fn test_keys() -> Vec<[u8; 32]> {
        vec![decode_key(TEST_PUBLIC).unwrap()]
    }

    /// `body` with the byte at `i` changed.
    fn flip(body: &[u8], i: usize) -> Vec<u8> {
        let mut b = body.to_vec();
        b[i] ^= 1;
        b
    }

    #[test]
    fn release_keys_are_valid_ed25519_keys() {
        assert!(!RELEASE_KEYS.is_empty());
        for k in RELEASE_KEYS {
            assert!(decode_key(k).is_some(), "{k}");
        }
        assert_eq!(
            trusted_keys_for(DEFAULT_URL, None).len(),
            RELEASE_KEYS.len()
        );
        // the test key is not a release key
        assert_eq!(&test_pair().pk[..], &decode_key(TEST_PUBLIC).unwrap()[..]);
        assert!(!RELEASE_KEYS.contains(&TEST_PUBLIC));
    }

    #[test]
    fn manifests_need_a_valid_signature() {
        let body = GOOD.as_bytes();
        let sig = sign(body);
        let f = open_signed(body, &sig, &test_keys(), false).unwrap();
        assert_eq!(f.manifest.version, "0.2.0");
        assert_eq!(
            (f.body.as_str(), f.signature.as_str()),
            (GOOD, sig.as_str())
        );
        // whitespace around and inside (openssl base64 wraps at 64) is fine
        let wrapped = format!("{}\n{}\n", &sig[..64], &sig[64..]);
        assert!(open_signed(body, &wrapped, &test_keys(), false).is_ok());
        let invalid = Err(SIG_INVALID.to_string());
        // any changed byte of the manifest: the version, a digest, the last
        for i in [
            GOOD.find("0.2.0").unwrap(),
            GOOD.find("ABCDEF").unwrap(),
            body.len() - 1,
        ] {
            assert_eq!(
                open_signed(&flip(body, i), &sig, &test_keys(), false),
                invalid,
                "byte {i}"
            );
        }
        // a changed signature byte (decoded), a truncated or empty signature
        let raw = test_pair().sk.sign(body, None);
        for i in [0, 31, 32, 63] {
            let bad = b64(&flip(&raw[..], i));
            assert_eq!(
                open_signed(body, &bad, &test_keys(), false),
                invalid,
                "sig byte {i}"
            );
        }
        for bad in ["", "nope!", &sig[..40], "AAAA"] {
            assert_eq!(
                open_signed(body, bad, &test_keys(), false),
                invalid,
                "{bad:?}"
            );
        }
        // another key's signature
        assert_eq!(
            open_signed(body, &sig, &trusted_keys_for(DEFAULT_URL, None), false),
            invalid
        );
        // a valid signature over an invalid manifest is still refused
        let evil = GOOD.replace(DL, "https://example.com/x");
        assert!(
            open_signed(evil.as_bytes(), &sign(evil.as_bytes()), &test_keys(), false)
                .unwrap_err()
                .starts_with("malformed manifest")
        );
    }

    #[test]
    fn the_test_key_is_only_trusted_for_file_manifests() {
        let release = trusted_keys_for(DEFAULT_URL, None);
        let test = decode_key(TEST_PUBLIC).unwrap();
        for url in [
            DEFAULT_URL,
            "https://example.com/latest.json",
            "http://x/latest.json",
        ] {
            let keys = trusted_keys_for(url, Some(TEST_PUBLIC));
            assert_eq!(keys, release, "{url}");
            assert_eq!(
                open_signed(GOOD.as_bytes(), &sign(GOOD.as_bytes()), &keys, false),
                Err(SIG_INVALID.to_string()),
                "{url}"
            );
        }
        let file = trusted_keys_for("file:///tmp/latest.json", Some(TEST_PUBLIC));
        assert!(file.contains(&test) && release.iter().all(|k| file.contains(k)));
        // a malformed test key adds nothing
        assert_eq!(
            trusted_keys_for("file:///tmp/latest.json", Some("short")),
            release
        );
    }

    #[test]
    fn fetch_verifies_the_signature_first() {
        let dir = scratch("fetch");
        let path = dir.join("latest.json");
        let url = format!("file://{}", path.display());
        let sig_path = dir.join("latest.json.sig");
        std::fs::write(&path, GOOD).unwrap();
        // no signature next to it
        assert_eq!(fetch_with(&url, &test_keys()), Err(SIG_MISSING.to_string()));
        std::fs::write(&sig_path, sign(GOOD.as_bytes()) + "\n").unwrap();
        assert_eq!(
            fetch_with(&url, &test_keys()).unwrap().manifest.version,
            "0.2.0"
        );
        // only the release keys (an https manifest's keys)
        assert_eq!(
            fetch_with(&url, &trusted_keys_for(DEFAULT_URL, None)),
            Err(SIG_INVALID.to_string())
        );
        // the manifest changed after signing
        std::fs::write(&path, GOOD.replace("0.2.0", "9.9.9")).unwrap();
        assert_eq!(fetch_with(&url, &test_keys()), Err(SIG_INVALID.to_string()));
        // an oversized signature file
        std::fs::write(&path, GOOD).unwrap();
        std::fs::write(&sig_path, "A".repeat(MAX_SIG + 1)).unwrap();
        assert_eq!(fetch_with(&url, &test_keys()), Err(SIG_INVALID.to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_round_trips_and_schedules() {
        let dir = scratch("cache");
        let path = dir.join("update.json");
        let keys = test_keys();
        let load = |p: &Path| load_cache_with(p, &keys, false);
        assert_eq!(load(&path), Cache::default());
        let mut c = Cache {
            checked_at: 1_000,
            dismissed: Some("0.2.0".into()),
            ..Cache::default()
        };
        c.store(&open_signed(GOOD.as_bytes(), &sign(GOOD.as_bytes()), &keys, false).unwrap());
        save_cache_to(&path, &c).unwrap();
        assert_eq!(load(&path), c);
        assert_eq!(load(&path).latest.unwrap().version, "0.2.0");
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let six_h = CHECK_EVERY.as_secs();
        assert!(!c.due(1_000 + six_h - 1));
        assert_eq!(c.due_in(1_000 + 10), six_h - 10);
        assert!(c.due(1_000 + six_h));
        assert!(c.due(500), "a clock that went backwards rechecks");
        assert!(Cache::default().due(now()));
        // the file is not trusted more than the network: a cached manifest
        // that does not verify or validate now is dropped (and rechecked)
        let dropped = |bad: &Cache| {
            save_cache_to(&path, bad).unwrap();
            let loaded = load(&path);
            (
                loaded.latest,
                loaded.manifest,
                loaded.signature,
                loaded.checked_at,
            )
        };
        let gone = (None, None, None, 0);
        let mut bad = c.clone();
        bad.manifest = Some(GOOD.replace("0.2.0", "9.9.9"));
        assert_eq!(dropped(&bad), gone, "tampered manifest");
        let mut bad = c.clone();
        bad.signature = Some(b64(&flip(
            &test_pair().sk.sign(GOOD.as_bytes(), None)[..],
            5,
        )));
        assert_eq!(dropped(&bad), gone, "tampered signature");
        let mut bad = c.clone();
        bad.signature = None;
        assert_eq!(dropped(&bad), gone, "no signature");
        // signed by a key not trusted for the current manifest URL
        save_cache_to(&path, &c).unwrap();
        let loaded = load_cache_with(&path, &trusted_keys_for(DEFAULT_URL, None), false);
        assert_eq!((loaded.latest, loaded.checked_at), (None, 0));
        // signed, but no longer valid
        let evil = GOOD.replace(DL, "file:///tmp");
        let mut bad = c.clone();
        bad.manifest = Some(evil.clone());
        bad.signature = Some(sign(evil.as_bytes()));
        assert_eq!(
            dropped(&bad),
            gone,
            "signed file:// asset without the override"
        );
        // an older cache with an unsigned parsed manifest injects nothing
        std::fs::write(
            &path,
            format!(r#"{{"checked_at":1000,"latest":{GOOD},"dismissed":"0.2.0"}}"#),
        )
        .unwrap();
        let loaded = load(&path);
        assert_eq!((loaded.latest, loaded.checked_at), (None, 0));
        assert_eq!(loaded.dismissed.as_deref(), Some("0.2.0"));
        let mut bad = c.clone();
        bad.dismissed = Some("../x".into());
        save_cache_to(&path, &bad).unwrap();
        assert_eq!(load(&path).dismissed, None);
        // a corrupt file is an empty cache, not an error
        std::fs::write(&path, b"{nope").unwrap();
        assert_eq!(load(&path), Cache::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_targets() {
        let yes = |_: &Path| DirAccess::Writable;
        let no = |_: &Path| DirAccess::Denied;
        let ro = |_: &Path| DirAccess::ReadOnlyVolume;
        let bundle = Path::new("/Applications/Rift.app/Contents/MacOS/rift-terminal");
        assert_eq!(
            install_target_for("macos", bundle, None, None, yes),
            InstallTarget::MacApp("/Applications/Rift.app".into())
        );
        assert!(matches!(
            install_target_for("macos", bundle, None, None, no),
            InstallTarget::Unsupported(r) if r.contains("not writable")
        ));
        // a mounted disk image, or Gatekeeper's translocated copy
        assert_eq!(
            install_target_for("macos", bundle, None, None, ro),
            InstallTarget::Unsupported(MOVE_TO_APPLICATIONS.into())
        );
        let translocated = Path::new(
            "/private/var/folders/x/T/AppTranslocation/1234/d/Rift.app/Contents/MacOS/rift-terminal",
        );
        assert_eq!(
            install_target_for("macos", translocated, None, None, yes),
            InstallTarget::Unsupported(MOVE_TO_APPLICATIONS.into())
        );
        for dev in [
            "/src/rift/target/debug/rift-terminal",
            "/usr/local/bin/rift-terminal",
            "/x/Contents/MacOS/rift-terminal",
        ] {
            assert!(matches!(
                install_target_for("macos", Path::new(dev), None, None, yes),
                InstallTarget::Unsupported(_)
            ));
        }
        let img = Path::new("/home/u/Apps/Rift-0.1.0-x86_64.AppImage");
        let appdir = Path::new("/tmp/.mount_x");
        let inside = Path::new("/tmp/.mount_x/usr/bin/rift-terminal");
        assert_eq!(
            install_target_for("linux", inside, Some(img), Some(appdir), yes),
            InstallTarget::AppImage(img.into())
        );
        // $APPIMAGE inherited by a shell's program: not this binary's image
        assert!(matches!(
            install_target_for(
                "linux",
                Path::new("/usr/bin/rift-terminal"),
                Some(img),
                Some(appdir),
                yes
            ),
            InstallTarget::Unsupported(_)
        ));
        assert!(matches!(
            install_target_for("linux", inside, Some(img), None, yes),
            InstallTarget::Unsupported(_)
        ));
        assert!(matches!(
            install_target_for(
                "linux",
                Path::new("/usr/bin/rift-terminal"),
                None,
                None,
                yes
            ),
            InstallTarget::Unsupported(_)
        ));
        assert!(matches!(
            install_target_for("linux", inside, Some(img), Some(appdir), no),
            InstallTarget::Unsupported(_)
        ));
        let mac = InstallTarget::MacApp("/A/Rift.app".into());
        let image = InstallTarget::AppImage("/A/S.AppImage".into());
        assert_eq!(old_path_of(&mac), Some("/A/.Rift-old.app".into()));
        assert_eq!(old_path_of(&image), Some("/A/S.AppImage.old".into()));
        assert_eq!(
            executable_of(&mac),
            Some("/A/Rift.app/Contents/MacOS/rift-terminal".into())
        );
        // a marker's paths are acted on only when they are this install's
        let p = Pending {
            from: "0.1.0".into(),
            to: "0.2.0".into(),
            old_path: "/A/.Rift-old.app".into(),
            new_path: "/A/Rift.app".into(),
            token: String::new(),
        };
        assert!(pending_matches_target(&p, &mac));
        assert!(!pending_matches_target(&p, &image));
        let evil = Pending {
            old_path: "/Users/u/Documents".into(),
            ..p.clone()
        };
        assert!(!pending_matches_target(&evil, &mac));
    }

    #[test]
    fn restart_tokens_expire_and_must_match() {
        let t0 = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let p = Pending {
            from: "0.1.0".into(),
            to: "0.2.0".into(),
            old_path: "/o".into(),
            new_path: "/n".into(),
            token: new_token().unwrap(),
        };
        assert_eq!(p.token.len(), 32);
        assert_ne!(p.token, new_token().unwrap());
        assert!(token_valid(&p, &p.token, t0, t0 + Duration::from_secs(59)));
        assert!(!token_valid(&p, &p.token, t0, t0 + TOKEN_TTL));
        assert!(!token_valid(&p, "", t0, t0));
        assert!(!token_valid(&p, "0000", t0, t0));
        let empty = Pending {
            token: String::new(),
            ..p.clone()
        };
        assert!(!token_valid(&empty, "", t0, t0));
        // the marker round-trips and is private
        let dir = scratch("marker");
        let path = dir.join("update-pending.json");
        write_pending(&path, &p).unwrap();
        assert_eq!(read_pending(&path), Some(p));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relaunch_drops_the_script_and_launch_services_args() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(relaunch_args(&[], None).is_empty());
        assert_eq!(
            relaunch_args(&s(&["--script", "a.rift", "--headless"]), None),
            Vec::<String>::new()
        );
        assert_eq!(
            relaunch_args(&s(&["--script", "a.rift", "--headless"]), Some("b.rift")),
            s(&["--headless", "--script", "b.rift"])
        );
        assert_eq!(
            relaunch_args(&s(&["-psn_0_12345"]), None),
            Vec::<String>::new()
        );
    }

    #[test]
    fn digests_and_swaps() {
        let dir = scratch("swap");
        let f = dir.join("f");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(sha256_file(&dir.join("missing")).is_err());
        // swap: current → old, staged → current (files: AppImage)
        let (cur, staged, old) = (dir.join("cur"), dir.join("staged"), dir.join("old"));
        std::fs::write(&cur, b"v1").unwrap();
        std::fs::write(&staged, b"v2").unwrap();
        swap(&cur, &staged, &old).unwrap();
        assert_eq!(std::fs::read(&cur).unwrap(), b"v2");
        assert_eq!(std::fs::read(&old).unwrap(), b"v1");
        assert!(!staged.exists());
        // a missing staged copy leaves the current app in place
        assert!(swap(&cur, &dir.join("nothing"), &old).is_err());
        assert_eq!(std::fs::read(&cur).unwrap(), b"v2");
        // and undone: the old one is back, nothing else is left
        let p = Pending {
            from: "1".into(),
            to: "2".into(),
            old_path: old.clone(),
            new_path: cur.clone(),
            token: String::new(),
        };
        std::fs::write(&old, b"v1").unwrap();
        restore_old(&p).unwrap();
        assert_eq!(std::fs::read(&cur).unwrap(), b"v1");
        assert!(!old.exists());
        // directories (Rift.app)
        let (app, new, bak) = (
            dir.join("S.app"),
            dir.join(".S-update.app"),
            dir.join(".S-old.app"),
        );
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("v"), b"1").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("v"), b"2").unwrap();
        swap(&app, &new, &bak).unwrap();
        assert_eq!(std::fs::read(app.join("v")).unwrap(), b"2");
        assert_eq!(std::fs::read(bak.join("v")).unwrap(), b"1");
        assert!(!new.exists());
        let p = Pending {
            old_path: bak.clone(),
            new_path: app.clone(),
            ..p
        };
        restore_old(&p).unwrap();
        assert_eq!(std::fs::read(app.join("v")).unwrap(), b"1");
        assert!(!bak.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inherited_tool_pipes_time_out() {
        let start = std::time::Instant::now();
        let result = run_for(
            std::process::Command::new("/bin/sh").args(["-c", "sleep 2 & exit 0"]),
            Duration::from_millis(100),
        );
        assert!(result.unwrap_err().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn slow_tools_time_out() {
        let t = std::time::Instant::now();
        let e = run_for(
            std::process::Command::new("/bin/sleep").arg("10"),
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(e.contains("timed out"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(5));
        assert_eq!(
            run_for(
                std::process::Command::new("/bin/echo").arg("hi"),
                Duration::from_secs(5)
            )
            .unwrap(),
            "hi\n"
        );
        assert!(
            run_for(
                &mut std::process::Command::new("/usr/bin/false"),
                Duration::from_secs(5)
            )
            .is_err()
        );
    }

    #[test]
    fn install_guard_is_released_when_a_fork_inherits_the_handle() {
        let dir = scratch("lock-fork");
        let lock = try_lock(&dir).unwrap();
        // SAFETY: the child only calls async-signal-safe libc functions and
        // exits without touching Rust state inherited from the test threads.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            unsafe {
                libc::sleep(1);
                libc::_exit(0);
            }
        }
        drop(lock);
        let again = try_lock(&dir);
        // Reap this test's child before asserting, including on failure.
        loop {
            // SAFETY: pid is the child created above; no user process is used.
            if unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) } >= 0
                || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                break;
            }
        }
        drop(again.expect("a forked child must not retain the released guard"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_install_lock_has_only_one_concurrent_successor() {
        let dir = scratch("lock-race");
        std::fs::create_dir_all(&dir).unwrap();
        for round in 0..32 {
            // Other parallel tests fork: a child can briefly inherit the
            // previous round's guard before exec closes it. Use a fresh inode.
            let dir = dir.join(round.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("install.lock"), "999999999").unwrap();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
            let threads: Vec<_> = (0..16)
                .map(|_| {
                    let dir = dir.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        let lock = try_lock(&dir).ok();
                        // Every winner stays alive until all contenders have tried.
                        barrier.wait();
                        lock.is_some()
                    })
                })
                .collect();
            let winners = threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .filter(|won| *won)
                .count();
            assert_eq!(winners, 1, "stale takeover must have exactly one owner");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_install_at_a_time() {
        let dir = scratch("lock");
        let lock = try_lock(&dir).unwrap();
        assert_eq!(
            try_lock(&dir).unwrap_err(),
            LockError::Held(std::process::id())
        );
        // another process (pid 1 is always alive) holding it blocks
        std::fs::write(dir.join("install.lock"), "1").unwrap();
        assert_eq!(try_lock(&dir).unwrap_err(), LockError::Held(1));
        // a dead holder's lock is stale: taken over
        std::fs::write(dir.join("install.lock"), "999999999").unwrap();
        drop(lock);
        std::fs::write(dir.join("install.lock"), "999999999").unwrap();
        let again = try_lock(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("install.lock")).unwrap(),
            std::process::id().to_string()
        );
        drop(again);
        assert!(!dir.join("install.lock").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_removes_interrupted_installs() {
        let dir = scratch("sweep");
        let updates = dir.join("updates");
        std::fs::create_dir_all(updates.join("download.mnt42")).unwrap();
        std::fs::write(updates.join("download.dmg"), b"partial").unwrap();
        std::fs::write(updates.join("relaunch.log"), b"keep").unwrap();
        let app = dir.join("Rift.app");
        std::fs::create_dir_all(dir.join(".Rift-update.app/Contents")).unwrap();
        sweep(&updates, &InstallTarget::MacApp(app));
        assert!(!updates.join("download.mnt42").exists());
        assert!(!updates.join("download.dmg").exists());
        assert!(updates.join("relaunch.log").exists());
        assert!(!dir.join(".Rift-update.app").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downloads_are_verified() {
        let dir = scratch("dl");
        let src = dir.join("src.bin");
        std::fs::write(&src, b"abc").unwrap();
        let good = Asset {
            url: format!("file://{}", src.display()),
            sha256: "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD".into(),
            kind: "dmg".into(),
        };
        let out = download_path(&dir.join("dl"), &good);
        assert_eq!(out, dir.join("dl/download.dmg"));
        download_with(&good, &out, true, MAX_ASSET).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"abc");
        // larger than allowed (no Content-Length to stop curl): deleted
        let e = download_with(&good, &out, true, 2).unwrap_err();
        assert!(e.contains("too large"), "{e}");
        assert!(!out.exists());
        // a wrong digest deletes the download
        let bad = Asset {
            sha256: "0".repeat(64),
            ..good.clone()
        };
        let e = download_with(&bad, &out, true, MAX_ASSET).unwrap_err();
        assert!(e.contains("SHA-256 mismatch"), "{e}");
        assert!(!out.exists());
        // file:// only under the test override; http never
        assert!(download_with(&good, &out, false, MAX_ASSET).is_err());
        let http = Asset {
            url: "http://example.com/x".into(),
            ..good.clone()
        };
        assert!(download_with(&http, &out, true, MAX_ASSET).is_err());
        // a missing file is a failed download, not a panic
        let gone = Asset {
            url: format!("file://{}", dir.join("nope").display()),
            ..good
        };
        assert!(download_with(&gone, &out, true, MAX_ASSET).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefetch_leaves_only_a_verified_ready_file() {
        let dir = scratch("prefetch");
        let src = dir.join("src.bin");
        std::fs::write(&src, b"abc").unwrap();
        let good = Asset {
            url: format!("file://{}", src.display()),
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
            kind: "dmg".into(),
        };
        let updates = dir.join("updates");
        std::fs::create_dir_all(&updates).unwrap();
        assert_eq!(ready_for(&updates, &good), None, "nothing downloaded yet");
        let ready = prefetch_with(&updates, &good, true, MAX_ASSET).unwrap();
        assert_eq!(ready, updates.join("ready.dmg"));
        assert_eq!(std::fs::read(&ready).unwrap(), b"abc");
        assert_eq!(ready_for(&updates, &good), Some(ready.clone()));
        // no partial file stays behind, and `sweep` (an install) keeps it
        let names = |d: &Path| {
            let mut v: Vec<String> = std::fs::read_dir(d)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        assert_eq!(names(&updates), ["ready.dmg"]);
        sweep(&updates, &InstallTarget::Unsupported(String::new()));
        assert!(ready.exists());
        // an intact ready file is taken as it is: the source can be gone
        std::fs::remove_file(&src).unwrap();
        assert_eq!(
            prefetch_with(&updates, &good, true, MAX_ASSET).unwrap(),
            ready
        );
        // another release's digest: the old file is not it, and a failed
        // fetch leaves no partial file
        let next = Asset {
            sha256: "0".repeat(64),
            ..good.clone()
        };
        assert_eq!(ready_for(&updates, &next), None);
        assert!(prefetch_with(&updates, &next, true, MAX_ASSET).is_err());
        assert_eq!(names(&updates), ["ready.dmg"]);
        // a corrupted ready file is never offered
        std::fs::write(&ready, b"abd").unwrap();
        assert_eq!(ready_for(&updates, &good), None);
        // up to date: what waits is deleted, nothing else
        std::fs::write(updates.join("relaunch.log"), b"").unwrap();
        drop_ready(&updates);
        assert_eq!(names(&updates), ["relaunch.log"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn run_helper(dir: &Path, pid: &str) -> std::process::ExitStatus {
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(HELPER)
            .arg("sh")
            .arg(pid)
            .args(["app", "app.old", "run.sh", "marker", "failed", "log"].map(|f| dir.join(f)))
            .status()
            .unwrap()
    }

    /// Written by a child shell, not this process: a test thread forking
    /// while this process held the file open for writing would make
    /// executing it fail with ETXTBSY (Linux), which reads as a crash.
    fn write_exe(dir: &Path, body: &str) {
        let exe = dir.join("run.sh");
        let ok = std::process::Command::new("/bin/sh")
            .args(["-c", r#"printf '#!/bin/sh\n%s\n' "$2" >"$1.tmp" && chmod 755 "$1.tmp" && mv -f "$1.tmp" "$1""#, "sh"])
            .arg(&exe)
            .arg(body)
            .status()
            .unwrap()
            .success();
        assert!(ok, "writing {}", exe.display());
    }

    #[test]
    fn helper_rolls_back_when_the_new_version_dies() {
        // the helper waits for a pid, starts `exe`, and restores `old` over
        // `target` when the new process dies with the marker still there
        let dir = scratch("helper");
        let (target, old, marker, failed) = (
            dir.join("app"),
            dir.join("app.old"),
            dir.join("marker"),
            dir.join("failed"),
        );
        std::fs::write(&target, "new").unwrap();
        std::fs::write(&old, "old").unwrap();
        std::fs::write(&marker, "{}").unwrap();
        // the "app": records which copy ran, then exits at once (a crash)
        write_exe(
            &dir,
            &format!("cat {} >> {}", target.display(), dir.join("ran").display()),
        );
        assert!(run_helper(&dir, "999999999").success()); // not running: start at once
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
        assert!(!old.exists() && !marker.exists() && failed.exists());
        assert!(!dir.join("app.failed").exists());
        assert_eq!(std::fs::read_to_string(dir.join("ran")).unwrap(), "newold");
        // a start that clears the marker is left alone
        std::fs::write(&target, "new").unwrap();
        std::fs::write(&old, "old").unwrap();
        std::fs::write(&marker, "{}").unwrap();
        write_exe(&dir, &format!("rm {}\nsleep 1", marker.display()));
        assert!(run_helper(&dir, "999999999").success());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(old.exists());
        // a parent that already exited but is not reaped (a zombie) counts
        // as gone; a log over 1 MB starts over
        let mut zombie = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(dir.join("log"), vec![b'x'; 1_100_000]).unwrap();
        std::fs::write(&marker, "{}").unwrap();
        write_exe(&dir, &format!("rm {}", marker.display()));
        let t = std::time::Instant::now();
        assert!(run_helper(&dir, &zombie.id().to_string()).success());
        assert!(t.elapsed() < Duration::from_secs(5), "waited on a zombie");
        assert!(std::fs::metadata(dir.join("log")).unwrap().len() < 1_000);
        let _ = zombie.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn helper_keeps_a_version_that_confirmed_then_quit() {
        // the new version clears the marker and exits at once (quit right
        // after confirming): the helper may see the marker, then the process
        // gone — it must not roll back an update that confirmed
        let dir = scratch("helper-confirm-race");
        let (target, old, marker) = (dir.join("app"), dir.join("app.old"), dir.join("marker"));
        write_exe(&dir, &format!("rm {}", marker.display()));
        for round in 0..40 {
            std::fs::write(&target, "new").unwrap();
            std::fs::write(&old, "old").unwrap();
            std::fs::write(&marker, "{}").unwrap();
            assert!(run_helper(&dir, "999999999").success(), "round {round}");
            assert_eq!(
                std::fs::read_to_string(&target).unwrap(),
                "new",
                "rolled back in round {round}"
            );
            assert!(!dir.join("failed").exists(), "round {round}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn helper_reads_process_state_without_ps() {
        // a system without `ps` (minimal images, toolbox containers): the
        // helper must still wait for the old process and must not take a
        // running new version for a crash and roll it back
        let dir = scratch("helper-no-ps");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for tool in ["wc", "sleep", "mv", "rm", "sed", "cat"] {
            let src = ["/usr/bin", "/bin"]
                .iter()
                .map(|d| Path::new(d).join(tool))
                .find(|p| p.exists())
                .unwrap();
            std::os::unix::fs::symlink(src, bin.join(tool)).unwrap();
        }
        let (target, old, marker) = (dir.join("app"), dir.join("app.old"), dir.join("marker"));
        std::fs::write(&target, "new").unwrap();
        std::fs::write(&old, "old").unwrap();
        std::fs::write(&marker, "{}").unwrap();
        // the new version confirms after a while, like a real start
        write_exe(&dir, &format!("sleep 1\nrm {}\nsleep 1", marker.display()));
        let mut parent = std::process::Command::new("/bin/sleep")
            .arg("1")
            .spawn()
            .unwrap();
        let t = std::time::Instant::now();
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(HELPER)
            .arg("sh")
            .arg(parent.id().to_string())
            .args(["app", "app.old", "run.sh", "marker", "failed", "log"].map(|f| dir.join(f)))
            .env("PATH", &bin)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(
            t.elapsed() >= Duration::from_millis(900),
            "did not wait for the old process"
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "new",
            "rolled back"
        );
        assert!(old.exists() && !dir.join("failed").exists());
        let _ = parent.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
