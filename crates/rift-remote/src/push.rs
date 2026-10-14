//! Finish alerts while the app is closed (`docs/remote-protocol.md` § Push).
//!
//! A device that wants them sends `push` with its Expo push token, an ntfy
//! topic URL, or both; the gateway keeps them in `devices.json`. For every
//! `done` it then decides (`alerts`) which devices get an alert: those that
//! asked about that pane (`notify`, one-shot, `Watches`) and those whose
//! `long_ms` the command reached (only a failed one, for `failures_only`). A
//! `Pusher` thread posts them with `curl` (as the updater fetches), at most
//! [`PUSH_RATE`] per device per minute: to the Expo push service (dropping a
//! token it reports as `DeviceNotRegistered`) and to each ntfy topic URL
//! (`ntfy_request`).
//!
//! An alert carries the host's name, the exit status, the host key and pane
//! (to open it on a tap) and, only when the device asked for it, the command
//! line; never output.

use crate::registry::{self, Device};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where alerts go (the Expo push service).
pub const EXPO_PUSH_URL: &str = "https://exp.host/--/api/v2/push/send";
/// Longest push token accepted.
pub const MAX_TOKEN: usize = 200;
/// Panes one device may be waiting on (`notify`).
pub const MAX_WATCHES: usize = 32;
/// A `notify` nobody's command answered is forgotten after this long.
pub const WATCH_TTL: Duration = Duration::from_secs(24 * 3600);
/// Alerts per device per minute; more are dropped.
pub const PUSH_RATE: usize = 12;
/// Longest `long_ms` accepted (a day).
pub const MAX_LONG_MS: u64 = 24 * 3600 * 1000;
/// Alert batches waiting for the sender; more are dropped.
const QUEUE: usize = 64;
/// One post to the push service may take this long.
const POST_TIMEOUT_SECS: &str = "15";
/// Connecting to it may take this long (a dead host fails fast).
const CONNECT_TIMEOUT_SECS: &str = "5";
/// Longest ntfy topic URL accepted.
pub const MAX_NTFY_URL: usize = 200;
/// Longest ntfy topic name accepted.
pub const MAX_NTFY_TOPIC: usize = 64;
/// Longest ntfy message body sent: ntfy turns a longer one into an
/// attachment.
const MAX_NTFY_BODY: usize = 4096;

/// An ntfy topic URL as the app sends it: `https://<host>[:port]/<topic>`
/// with a topic of `[A-Za-z0-9_-]{1,64}`, at most [`MAX_NTFY_URL`] bytes,
/// and no user, query or fragment.
pub fn valid_ntfy(url: &str) -> bool {
    if url.len() > MAX_NTFY_URL {
        return false;
    }
    let Some((authority, topic)) = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split_once('/'))
    else {
        return false;
    };
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    let host_ok = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !host.starts_with(['-', '.'])
        && !host.ends_with(['-', '.'])
        && !host.contains("..");
    let port_ok = port.is_none_or(|p| {
        !p.is_empty()
            && p.len() <= 5
            && p.bytes().all(|b| b.is_ascii_digit())
            && p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n))
    });
    let topic_ok = !topic.is_empty()
        && topic.len() <= MAX_NTFY_TOPIC
        && topic
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    host_ok && port_ok && topic_ok && public_enough(host)
}

/// Whether the gateway may post to `host` (already a valid host name or
/// IPv4 address): never to `localhost` / `*.localhost`, nor to an IPv4
/// address that is loopback, unspecified (0/8), link-local, broadcast or
/// multicast. Private LAN and 100.64/10 (Tailscale) addresses are allowed:
/// a self-hosted ntfy lives there. A name that resolves to an internal
/// address is not caught (`docs/mobile-threat-model.md`).
fn public_enough(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return false;
    }
    // A numeric last label makes the whole host an IPv4 address to curl
    // (WHATWG / inet_aton: `127.1`, `2130706433`, `0x7f.1`): only a plain
    // dotted quad is accepted then.
    let last = lower.rsplit('.').next().unwrap_or("");
    let numeric = last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x");
    if !numeric {
        return true;
    }
    let Ok(ip) = lower.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    !(ip.is_loopback()
        || ip.octets()[0] == 0
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast())
}

/// An Expo push token as the app sends it: `ExponentPushToken[…]` (or the
/// newer `ExpoPushToken[…]`) with a URL-safe inside.
pub fn valid_token(token: &str) -> bool {
    if token.len() > MAX_TOKEN {
        return false;
    }
    let inner = token
        .strip_prefix("ExponentPushToken[")
        .or_else(|| token.strip_prefix("ExpoPushToken["))
        .and_then(|t| t.strip_suffix(']'));
    inner.is_some_and(|i| {
        !i.is_empty()
            && i.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// The panes each device asked about (`notify`): one alert each, for the
/// next command that finishes there.
#[derive(Default)]
pub(crate) struct Watches {
    by_device: HashMap<String, HashMap<u64, Instant>>,
}

impl Watches {
    /// Wait on `pane` for `device`; false when it already waits on
    /// [`MAX_WATCHES`] other panes.
    pub(crate) fn add(&mut self, device: &str, pane: u64, now: Instant) -> bool {
        let panes = self.by_device.entry(device.to_string()).or_default();
        panes.retain(|_, at| now.duration_since(*at) < WATCH_TTL);
        if panes.len() >= MAX_WATCHES && !panes.contains_key(&pane) {
            return false;
        }
        panes.insert(pane, now);
        true
    }

    /// Whether `device` waited on `pane` (and no longer does); also
    /// `notify` with `off`.
    pub(crate) fn take(&mut self, device: &str, pane: u64, now: Instant) -> bool {
        let Some(panes) = self.by_device.get_mut(device) else {
            return false;
        };
        let hit = panes
            .remove(&pane)
            .is_some_and(|at| now.duration_since(at) < WATCH_TTL);
        if panes.is_empty() {
            self.by_device.remove(device);
        }
        hit
    }

    pub(crate) fn forget(&mut self, device: &str) {
        self.by_device.remove(device);
    }
}

/// A finished command, as `done` reports it.
pub(crate) struct Finished<'a> {
    pub pane: u64,
    pub command: &'a str,
    pub exit: Option<i32>,
    pub duration_ms: u64,
}

/// One alert for one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Alert {
    pub device: String,
    /// The Expo push token to send it to, if any.
    pub token: Option<String>,
    /// The ntfy topic URL to send it to, if any.
    pub ntfy: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub pane: u64,
    pub exit: Option<i32>,
    /// The device asked about this pane ("Notify me").
    pub asked: bool,
}

/// The alert's title (and body): the same words as the app's own
/// notifications.
pub fn alert_text(
    host: &str,
    exit: Option<i32>,
    command: &str,
    show_command: bool,
) -> (String, Option<String>) {
    let title = match exit {
        None | Some(0) => format!("A command finished on {host}"),
        Some(code) => format!("A command failed on {host} (exit {code})"),
    };
    (title, show_command.then(|| command.to_string()))
}

/// Which devices get an alert for `done`: those waiting on its pane (the
/// wait is used up) and those whose `long_ms` it reached (when it failed,
/// for `failures_only`).
pub(crate) fn alerts(
    done: &Finished,
    devices: &[Device],
    watches: &mut Watches,
    host: &str,
    now: Instant,
) -> Vec<Alert> {
    let mut out = Vec::new();
    for d in devices {
        let Some(reg) = d.alerts() else {
            continue;
        };
        let asked = watches.take(&d.id, done.pane, now);
        let failed = !matches!(done.exit, None | Some(0));
        let long =
            reg.long_ms.is_some_and(|ms| done.duration_ms >= ms) && (failed || !reg.failures_only);
        if !(asked || long) {
            continue;
        }
        let (title, body) = alert_text(host, done.exit, done.command, reg.show_command);
        out.push(Alert {
            device: d.id.clone(),
            token: reg.token.clone(),
            ntfy: reg.ntfy.clone(),
            title,
            body,
            pane: done.pane,
            exit: done.exit,
            asked,
        });
    }
    out
}

/// Drop the ntfy delivery of each alert nobody asked for ("Notify me") about
/// a pane its device follows right now (`followed`: device id and pane of
/// every live client): the user is looking at it, and the ntfy app's banner
/// cannot be suppressed by Rift. An Expo token keeps its delivery (the app
/// drops a foreground alert about the pane on screen itself); an alert left
/// with neither is removed.
pub(crate) fn skip_followed(alerts: &mut Vec<Alert>, followed: &HashSet<(String, u64)>) {
    alerts.retain_mut(|a| {
        if !a.asked && followed.contains(&(a.device.clone(), a.pane)) {
            a.ntfy = None;
        }
        a.token.is_some() || a.ntfy.is_some()
    });
}

/// The push service request for `alerts` (one message each; `data` opens
/// the pane on a tap). Only alerts with a token belong in it.
pub(crate) fn payload(alerts: &[&Alert], host_key: &str) -> String {
    let msgs: Vec<Value> = alerts
        .iter()
        .map(|a| {
            let mut m = json!({
                "to": a.token,
                "title": a.title,
                "data": {"host": host_key, "pane": a.pane},
                "priority": "high",
                "sound": "default",
            });
            if let Some(b) = &a.body {
                m["body"] = Value::String(b.clone());
            }
            m
        })
        .collect();
    Value::Array(msgs).to_string()
}

/// The app link that opens `pane` of the gateway with key `host_key`
/// (expo-router route `host/[key]/pane/[id]`, scheme `rift`).
pub fn pane_link(host_key: &str, pane: u64) -> String {
    format!(
        "rift://host/{}/pane/{pane}",
        registry::percent_encode(host_key)
    )
}

/// `value` as an HTTP header value: as is when it is printable ASCII, else
/// an RFC 2047 encoded word (ntfy decodes it; a host name like "Jo’s Mac"
/// is not ASCII, and a control character must never reach a header).
fn header_value(value: &str) -> String {
    if value.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return value.to_string();
    }
    use base64::Engine;
    format!(
        "=?UTF-8?B?{}?=",
        base64::engine::general_purpose::STANDARD.encode(value)
    )
}

/// The ntfy request for `alert`: its headers (`Name: value`) and its body,
/// the command line when the device asked for it, else "Exit N" or
/// "Finished" (`docs/remote-protocol.md` § Push).
pub(crate) fn ntfy_request(alert: &Alert, host_key: &str) -> (Vec<String>, String) {
    let failed = !matches!(alert.exit, None | Some(0));
    let mut body = match &alert.body {
        Some(command) if !command.is_empty() => command.clone(),
        _ => match alert.exit {
            Some(code) if failed => format!("Exit {code}"),
            _ => "Finished".to_string(),
        },
    };
    if body.len() > MAX_NTFY_BODY {
        let mut end = MAX_NTFY_BODY;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
    }
    let headers = vec![
        format!("Title: {}", header_value(&alert.title)),
        format!("Priority: {}", if failed { "high" } else { "default" }),
        format!("Tags: {}", if failed { "x" } else { "white_check_mark" }),
        format!("Click: {}", pane_link(host_key, alert.pane)),
    ];
    (headers, body)
}

/// The positions (in the request) of the alerts whose token the service
/// no longer knows.
pub(crate) fn unregistered(response: &str) -> Vec<usize> {
    let Ok(v) = serde_json::from_str::<Value>(response) else {
        return Vec::new();
    };
    v["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .enumerate()
                .filter(|(_, it)| it["details"]["error"] == "DeviceNotRegistered")
                .map(|(i, _)| i)
                .collect()
        })
        .unwrap_or_default()
}

/// The thread that posts alerts. Dropping it ends the thread.
pub(crate) struct Pusher {
    tx: SyncSender<Vec<Alert>>,
}

/// Where ntfy posts go instead of a device's topic URL (tests): a URL that
/// starts with `.0` is posted with that prefix replaced by `.1`.
pub(crate) type NtfyRedirect = Vec<(String, String)>;

impl Pusher {
    /// Start posting to `url` (Expo) and to each device's ntfy topic URL
    /// (`redirect`: tests) for the gateway with key `host_key`; `dir` is the
    /// registry (unregistered tokens are removed there).
    pub(crate) fn start(
        url: String,
        redirect: NtfyRedirect,
        host_key: String,
        dir: PathBuf,
    ) -> std::io::Result<Pusher> {
        let (tx, rx) = sync_channel(QUEUE);
        std::thread::Builder::new()
            .name("remote-push".into())
            .spawn(move || send_loop(&rx, &url, &redirect, &host_key, &dir))?;
        Ok(Pusher { tx })
    }

    /// Queue `alerts` (dropped, with a warning, when the queue is full).
    pub(crate) fn send(&self, alerts: Vec<Alert>) {
        if alerts.is_empty() {
            return;
        }
        if let Err(TrySendError::Full(_)) = self.tx.try_send(alerts) {
            log::warn!("remote: push queue full; alert dropped");
        }
    }
}

/// The devices with an ntfy post under way: at most one each, so a slow or
/// dead ntfy server holds up only the device that named it.
#[derive(Clone, Default)]
pub(crate) struct InFlight(Arc<Mutex<HashSet<String>>>);

/// One device's post under way; dropping it ends it.
pub(crate) struct Flight(InFlight, String);

impl InFlight {
    /// Start a post for `device`; `None` while one is still under way.
    pub(crate) fn begin(&self, device: &str) -> Option<Flight> {
        self.0
            .lock()
            .unwrap()
            .insert(device.to_string())
            .then(|| Flight(self.clone(), device.to_string()))
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().remove(&self.1);
    }
}

fn send_loop(
    rx: &Receiver<Vec<Alert>>,
    url: &str,
    redirect: &NtfyRedirect,
    host_key: &str,
    dir: &std::path::Path,
) {
    let mut sent: HashMap<String, VecDeque<Instant>> = HashMap::new();
    let flying = InFlight::default();
    while let Ok(batch) = rx.recv() {
        let now = Instant::now();
        let batch: Vec<Alert> = batch
            .into_iter()
            .filter(|a| {
                let log = sent.entry(a.device.clone()).or_default();
                while log
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
                {
                    log.pop_front();
                }
                let ok = log.len() < PUSH_RATE;
                if ok {
                    log.push_back(now);
                } else {
                    log::info!("remote: push to {} dropped (rate)", a.device);
                }
                ok
            })
            .collect();
        // ntfy first, a thread per device: never behind another device's
        // server, nor in front of the Expo post
        let mut by_device: HashMap<&str, Vec<&Alert>> = HashMap::new();
        for a in batch.iter().filter(|a| a.ntfy.is_some()) {
            by_device.entry(a.device.as_str()).or_default().push(a);
        }
        for (device, alerts) in by_device {
            post_ntfy(&flying, device, &alerts, redirect, host_key);
        }
        let expo: Vec<&Alert> = batch.iter().filter(|a| a.token.is_some()).collect();
        if !expo.is_empty() {
            let headers = ["Content-Type: application/json", "Accept: application/json"];
            match post(url, &headers, &payload(&expo, host_key)) {
                Ok(response) => {
                    for i in unregistered(&response) {
                        let Some(a) = expo.get(i) else { continue };
                        let Some(token) = &a.token else { continue };
                        log::info!("remote: push token of {} is no longer valid", a.device);
                        if let Err(e) = registry::clear_token(dir, &a.device, token) {
                            log::warn!("remote: push: {e}");
                        }
                    }
                }
                Err(e) => log::warn!("remote: push: {e}"),
            }
        }
    }
}

/// Post `device`'s `alerts` (one batch) to their ntfy topic, one after
/// another on a thread of their own; dropped (and logged) while the
/// device's previous batch is still under way.
fn post_ntfy(
    flying: &InFlight,
    device: &str,
    alerts: &[&Alert],
    redirect: &NtfyRedirect,
    host_key: &str,
) {
    let Some(flight) = flying.begin(device) else {
        log::info!(
            "remote: {} ntfy alert(s) to {device} dropped (the previous post is still under way)",
            alerts.len()
        );
        return;
    };
    let requests: Vec<(String, Vec<String>, String)> = alerts
        .iter()
        .filter_map(|a| {
            let topic = a.ntfy.as_deref()?;
            let (mut headers, body) = ntfy_request(a, host_key);
            headers.push("Content-Type: text/plain; charset=utf-8".into());
            let target = redirect
                .iter()
                .find_map(|(from, to)| {
                    topic
                        .strip_prefix(from.as_str())
                        .map(|rest| format!("{to}{rest}"))
                })
                .unwrap_or_else(|| topic.to_string());
            Some((target, headers, body))
        })
        .collect();
    let who = device.to_string();
    let spawned = std::thread::Builder::new()
        .name("remote-ntfy".into())
        .spawn(move || {
            let _flight = flight;
            for (target, headers, body) in requests {
                let headers: Vec<&str> = headers.iter().map(String::as_str).collect();
                // the reply is not read: ntfy reports nothing to act on
                if let Err(e) = post(&target, &headers, &body) {
                    log::warn!("remote: ntfy alert to {who}: {e}");
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("remote: ntfy alert to {device}: {e}");
    }
}

/// POST `body` with `headers` to `url` with curl: https only, except a
/// loopback `http://` address (tests). Returns the reply; never logs it.
fn post(url: &str, headers: &[&str], body: &str) -> Result<String, String> {
    let local = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
        .iter()
        .any(|p| url.starts_with(p));
    let protos = if local { "=http" } else { "=https" };
    let mut cmd = Command::new("curl");
    cmd.args(["-q", "-sS", "--max-time", POST_TIMEOUT_SECS])
        .args(["--connect-timeout", CONNECT_TIMEOUT_SECS])
        .args(["--proto", protos, "--proto-redir", protos]);
    for h in headers {
        cmd.args(["-H", h]);
    }
    let mut child = cmd
        .args(["--data-binary", "@-", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    // a failed write still waits for curl: returning early would leave it a
    // zombie for the gateway's life
    let written = match child.stdin.take() {
        Some(mut stdin) => stdin.write_all(body.as_bytes()),
        None => Ok(()),
    };
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    written.map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Permission;
    use crate::registry::PushReg;
    use std::time::SystemTime;

    fn device(id: &str, reg: Option<PushReg>) -> Device {
        let (push, ntfy) = reg.map(PushReg::split).unwrap_or_default();
        Device {
            ntfy,
            id: id.into(),
            name: format!("Phone {id}"),
            platform: "android".into(),
            key: "k".into(),
            permission: Permission::View,
            paired_at: SystemTime::UNIX_EPOCH,
            last_seen: SystemTime::UNIX_EPOCH,
            push,
        }
    }

    fn reg(long_ms: Option<u64>, show_command: bool) -> Option<PushReg> {
        Some(PushReg {
            token: Some("ExponentPushToken[abc]".into()),
            ntfy: None,
            long_ms,
            show_command,
            failures_only: false,
        })
    }

    #[test]
    fn failures_only_skips_long_successes_but_not_asked_panes() {
        let now = Instant::now();
        let mut failing = reg(Some(1_000), false);
        failing.as_mut().unwrap().failures_only = true;
        let devices = vec![device("f", failing), device("all", reg(Some(1_000), false))];
        let mut w = Watches::default();
        let who = |a: Vec<Alert>| a.into_iter().map(|a| a.device).collect::<Vec<_>>();
        // a long success (exit 0, or no status): only the device that wants all
        for exit in [Some(0), None] {
            let a = alerts(&done(1, 5_000, exit), &devices, &mut w, "Mac", now);
            assert_eq!(who(a), ["all"], "{exit:?}");
        }
        // a long failure: both
        let a = alerts(&done(1, 5_000, Some(2)), &devices, &mut w, "Mac", now);
        assert_eq!(who(a), ["f", "all"]);
        // a short failure: neither
        assert!(alerts(&done(1, 10, Some(2)), &devices, &mut w, "Mac", now).is_empty());
        // "Notify me" alerts a success too
        assert!(w.add("f", 3, now));
        let a = alerts(&done(3, 10, Some(0)), &devices, &mut w, "Mac", now);
        assert_eq!(who(a), ["f"]);
    }

    fn done(pane: u64, duration_ms: u64, exit: Option<i32>) -> Finished<'static> {
        Finished {
            pane,
            command: "make release",
            exit,
            duration_ms,
        }
    }

    #[test]
    fn tokens_are_expo_push_tokens() {
        for ok in ["ExponentPushToken[abc-_09XYZ]", "ExpoPushToken[q]"] {
            assert!(valid_token(ok), "{ok}");
        }
        for bad in [
            "",
            "ExponentPushToken[]",
            "ExponentPushToken[abc",
            "ExponentPushToken[a b]",
            "ExponentPushToken[a\"]",
            "fcm:abc",
            &format!("ExponentPushToken[{}]", "a".repeat(MAX_TOKEN)),
        ] {
            assert!(!valid_token(bad), "{bad}");
        }
    }

    #[test]
    fn long_commands_and_asked_panes_alert_others_do_not() {
        let now = Instant::now();
        let devices = vec![
            device("long", reg(Some(30_000), false)),
            device("asks", reg(None, true)),
            device("none", None),
        ];
        let mut w = Watches::default();
        // short, nobody asked: nothing
        assert!(alerts(&done(1, 5_000, Some(0)), &devices, &mut w, "Mac", now).is_empty());
        // long: only the device with long_ms
        let a = alerts(&done(1, 30_000, Some(0)), &devices, &mut w, "Mac", now);
        assert_eq!(
            a.iter().map(|a| a.device.as_str()).collect::<Vec<_>>(),
            ["long"]
        );
        assert_eq!(a[0].title, "A command finished on Mac");
        assert_eq!(a[0].body, None, "the command stays out unless asked");
        // asked: short commands too, once, with the command when wanted
        assert!(w.add("asks", 2, now));
        assert!(w.add("none", 2, now));
        let a = alerts(&done(2, 10, Some(3)), &devices, &mut w, "Mac", now);
        assert_eq!(a.len(), 1, "a device without a token gets nothing");
        assert_eq!(a[0].device, "asks");
        assert_eq!(a[0].title, "A command failed on Mac (exit 3)");
        assert_eq!(a[0].body.as_deref(), Some("make release"));
        assert!(alerts(&done(2, 10, Some(0)), &devices, &mut w, "Mac", now).is_empty());
        // another pane does not use up the wait
        assert!(w.add("asks", 3, now));
        assert!(alerts(&done(4, 10, Some(0)), &devices, &mut w, "Mac", now).is_empty());
        assert_eq!(
            alerts(&done(3, 10, None), &devices, &mut w, "Mac", now).len(),
            1
        );
    }

    #[test]
    fn waits_expire_and_are_bounded() {
        let t0 = Instant::now();
        let mut w = Watches::default();
        for pane in 0..MAX_WATCHES as u64 {
            assert!(w.add("d", pane, t0));
        }
        assert!(!w.add("d", 999, t0), "full");
        assert!(w.add("d", 0, t0), "an existing pane is renewed");
        let later = t0 + WATCH_TTL;
        assert!(w.add("d", 999, later), "expired waits make room");
        assert!(!w.take("d", 1, later), "an expired wait does not alert");
        assert!(w.take("d", 999, later));
    }

    #[test]
    fn the_request_and_the_unregistered_reply() {
        let a = Alert {
            device: "d".into(),
            token: Some("ExponentPushToken[abc]".into()),
            ntfy: None,
            title: "A command finished on Mac".into(),
            body: None,
            pane: 12,
            exit: Some(0),
            asked: false,
        };
        let v: Value = serde_json::from_str(&payload(&[&a], "HOSTKEY")).unwrap();
        assert_eq!(
            v,
            json!([{
                "to": "ExponentPushToken[abc]",
                "title": "A command finished on Mac",
                "data": {"host": "HOSTKEY", "pane": 12},
                "priority": "high",
                "sound": "default",
            }])
        );
        let reply = r#"{"data":[{"status":"ok","id":"x"},{"status":"error","message":"m","details":{"error":"DeviceNotRegistered"}},{"status":"error","details":{"error":"MessageRateExceeded"}}]}"#;
        assert_eq!(unregistered(reply), [1]);
        assert!(unregistered("not json").is_empty());
    }

    #[test]
    fn ntfy_urls_are_https_topic_urls() {
        let max_topic = format!("https://ntfy.sh/{}", "a".repeat(MAX_NTFY_TOPIC));
        for ok in [
            "https://ntfy.sh/rift-x7Kq2mP9vT4wL8nR3sY6hZ",
            "https://ntfy.example.com:8443/a_b-C",
            "https://10.0.0.2/t",
            "https://192.168.1.20:8080/t",
            "https://172.16.0.5/t",
            "https://100.101.102.103/t",
            "https://localhost.example.com/t",
            "https://ntfy.my-tailnet.ts.net/t",
            "https://8.8.8.8/t",
            max_topic.as_str(),
        ] {
            assert!(valid_ntfy(ok), "{ok}");
        }
        let long_host = format!("https://{}.com/t", "h".repeat(MAX_NTFY_URL));
        for bad in [
            "",
            "http://ntfy.sh/topic",
            "HTTPS://ntfy.sh/topic",
            "ntfy://ntfy.sh/topic",
            "https://ntfy.sh/",
            "https://ntfy.sh",
            "https:///topic",
            "https://user:pw@ntfy.sh/topic",
            "https://user@ntfy.sh/topic",
            "https://ntfy.sh/topic?auth=x",
            "https://ntfy.sh/topic#x",
            "https://ntfy.sh/a/b",
            "https://ntfy.sh/to pic",
            "https://ntfy.sh/topic\n",
            "https://ntfy.sh:0/topic",
            "https://ntfy.sh:65536/topic",
            "https://ntfy.sh:/topic",
            "https://-ntfy.sh/topic",
            "https://ntfy..sh/topic",
            "https://[::1]/topic",
            "https://ntfy.sh/tópico",
            // request forgery: local names and special IPv4 addresses
            "https://localhost/t",
            "https://LocalHost:8080/t",
            "https://ntfy.localhost/t",
            "https://127.0.0.1/t",
            "https://127.9.9.9:443/t",
            "https://0.0.0.0/t",
            "https://0.1.2.3/t",
            "https://169.254.169.254/t",
            "https://255.255.255.255/t",
            "https://224.0.0.1/t",
            "https://239.255.255.250/t",
            // other spellings curl reads as 127.0.0.1
            "https://127.1/t",
            "https://2130706433/t",
            "https://0x7f.0.0.1/t",
            "https://0177.0.0.1/t",
            "https://ntfy.256/t",
            &format!("https://ntfy.sh/{}", "a".repeat(MAX_NTFY_TOPIC + 1)),
            &long_host,
        ] {
            assert!(!valid_ntfy(bad), "{bad:?}");
        }
    }

    fn alert(exit: Option<i32>, body: Option<&str>, title: &str) -> Alert {
        Alert {
            device: "d".into(),
            token: None,
            ntfy: Some("https://ntfy.sh/t".into()),
            title: title.into(),
            body: body.map(Into::into),
            pane: 12,
            exit,
            asked: false,
        }
    }

    #[test]
    fn the_ntfy_request_has_the_title_priority_tags_and_pane_link() {
        let (h, body) = ntfy_request(
            &alert(Some(2), None, "A command failed on Mac (exit 2)"),
            "K-_9",
        );
        assert_eq!(
            h,
            [
                "Title: A command failed on Mac (exit 2)",
                "Priority: high",
                "Tags: x",
                "Click: rift://host/K-_9/pane/12",
            ]
        );
        assert_eq!(body, "Exit 2", "the command stays out unless asked");
        for exit in [Some(0), None] {
            let (h, body) = ntfy_request(&alert(exit, None, "A command finished on Mac"), "K");
            assert_eq!(h[1..3], ["Priority: default", "Tags: white_check_mark"]);
            assert_eq!(body, "Finished");
        }
        // with show_command, the command line is the body (cut at 4 KiB)
        let (_, body) = ntfy_request(&alert(Some(0), Some("make release"), "t"), "K");
        assert_eq!(body, "make release");
        let long = "é".repeat(3000);
        let (_, body) = ntfy_request(&alert(Some(0), Some(&long), "t"), "K");
        assert_eq!(body, "é".repeat(MAX_NTFY_BODY / 2));
        // a title that is not printable ASCII goes as an RFC 2047 word
        let (h, _) = ntfy_request(&alert(Some(0), None, "A command finished on Jo’s Mac"), "K");
        assert_eq!(
            h[0],
            "Title: =?UTF-8?B?QSBjb21tYW5kIGZpbmlzaGVkIG9uIEpv4oCZcyBNYWM=?="
        );
        let (h, _) = ntfy_request(&alert(Some(0), None, "evil\r\nX-Inject: 1"), "K");
        assert!(!h[0].contains(['\r', '\n']), "{}", h[0]);
        // the host key is percent-encoded into the link
        assert_eq!(pane_link("a/b+c", 3), "rift://host/a%2Fb%2Bc/pane/3");
    }

    #[test]
    fn a_device_with_only_an_ntfy_url_gets_alerts_by_the_same_rules() {
        let now = Instant::now();
        let ntfy = Some(PushReg {
            token: None,
            ntfy: Some("https://ntfy.sh/t".into()),
            long_ms: Some(1_000),
            show_command: false,
            failures_only: true,
        });
        let empty = Some(PushReg {
            token: None,
            ntfy: None,
            long_ms: Some(1_000),
            show_command: false,
            failures_only: false,
        });
        let devices = vec![device("n", ntfy), device("e", empty)];
        let mut w = Watches::default();
        assert!(alerts(&done(1, 5_000, Some(0)), &devices, &mut w, "Mac", now).is_empty());
        let a = alerts(&done(1, 5_000, Some(1)), &devices, &mut w, "Mac", now);
        assert_eq!(a.len(), 1, "a registration with neither gets nothing");
        assert_eq!(a[0].ntfy.as_deref(), Some("https://ntfy.sh/t"));
        assert_eq!(a[0].token, None);
        assert_eq!(a[0].exit, Some(1));
        assert!(!a[0].asked);
        assert!(w.add("n", 2, now));
        let a = alerts(&done(2, 10, Some(0)), &devices, &mut w, "Mac", now);
        assert!(a[0].asked, "a Notify me is marked asked");
    }

    #[test]
    fn one_ntfy_post_per_device_is_under_way_at_a_time() {
        let f = InFlight::default();
        let a = f.begin("a").expect("first");
        assert!(f.begin("a").is_none(), "a second while the first runs");
        let b = f.begin("b").expect("another device is not held up");
        drop(a);
        assert!(f.begin("a").is_some(), "free again once it ended");
        drop(b);
    }

    #[test]
    fn a_followed_pane_gets_no_ntfy_banner_unless_asked() {
        let mut both = alert(Some(1), None, "t");
        both.token = Some("ExponentPushToken[abc]".into());
        let mut asked = alert(Some(1), None, "t");
        asked.asked = true;
        let mut other_pane = alert(Some(1), None, "t");
        other_pane.pane = 13;
        let mut other_device = alert(Some(1), None, "t");
        other_device.device = "e".into();
        let mut list = vec![
            alert(Some(1), None, "t"),
            both,
            asked,
            other_pane,
            other_device,
        ];
        let followed: HashSet<(String, u64)> = [("d".to_string(), 12)].into();
        skip_followed(&mut list, &followed);
        let got: Vec<(String, u64, bool, bool)> = list
            .iter()
            .map(|a| {
                (
                    a.device.clone(),
                    a.pane,
                    a.token.is_some(),
                    a.ntfy.is_some(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                // the ntfy-only alert about the followed pane is gone;
                // the one with a token keeps only its Expo delivery
                ("d".to_string(), 12, true, false),
                ("d".to_string(), 12, false, true),
                ("d".to_string(), 13, false, true),
                ("e".to_string(), 12, false, true),
            ]
        );
    }
}
