//! Finish alerts while the app is closed (`docs/remote-protocol.md` § Push).
//!
//! A device that wants them sends `push` with its Expo push token; the
//! gateway keeps it in `devices.json`. For every `done` it then decides
//! ([`alerts`]) which devices get an alert: those that asked about that pane
//! (`notify`, one-shot, [`Watches`]) and those whose `long_ms` the command
//! reached. A [`Pusher`] thread posts them to the Expo push service with
//! `curl` (as the updater fetches), at most [`PUSH_RATE`] per device per
//! minute, and drops a token the service reports as `DeviceNotRegistered`.
//!
//! An alert carries the host's name, the exit status, the host key and pane
//! (to open it on a tap) and, only when the device asked for it, the command
//! line; never output.

use crate::registry::{self, Device};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
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

    /// Whether `device` waited on `pane` (and no longer does).
    fn take(&mut self, device: &str, pane: u64, now: Instant) -> bool {
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
    pub token: String,
    pub title: String,
    pub body: Option<String>,
    pub pane: u64,
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
/// wait is used up) and those whose `long_ms` it reached.
pub(crate) fn alerts(
    done: &Finished,
    devices: &[Device],
    watches: &mut Watches,
    host: &str,
    now: Instant,
) -> Vec<Alert> {
    let mut out = Vec::new();
    for d in devices {
        let Some(reg) = &d.push else {
            continue;
        };
        let asked = watches.take(&d.id, done.pane, now);
        let long = reg.long_ms.is_some_and(|ms| done.duration_ms >= ms);
        if !(asked || long) {
            continue;
        }
        let (title, body) = alert_text(host, done.exit, done.command, reg.show_command);
        out.push(Alert {
            device: d.id.clone(),
            token: reg.token.clone(),
            title,
            body,
            pane: done.pane,
        });
    }
    out
}

/// The push service request for `alerts` (one message each; `data` opens
/// the pane on a tap).
pub(crate) fn payload(alerts: &[Alert], host_key: &str) -> String {
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

impl Pusher {
    /// Start posting to `url` for the gateway with key `host_key`; `dir` is
    /// the registry (unregistered tokens are removed there).
    pub(crate) fn start(url: String, host_key: String, dir: PathBuf) -> std::io::Result<Pusher> {
        let (tx, rx) = sync_channel(QUEUE);
        std::thread::Builder::new()
            .name("remote-push".into())
            .spawn(move || send_loop(&rx, &url, &host_key, &dir))?;
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

fn send_loop(rx: &Receiver<Vec<Alert>>, url: &str, host_key: &str, dir: &std::path::Path) {
    let mut sent: HashMap<String, VecDeque<Instant>> = HashMap::new();
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
        if batch.is_empty() {
            continue;
        }
        match post(url, &payload(&batch, host_key)) {
            Ok(response) => {
                for i in unregistered(&response) {
                    let a = &batch[i];
                    log::info!("remote: push token of {} is no longer valid", a.device);
                    forget_token(dir, &a.device, &a.token);
                }
            }
            Err(e) => log::warn!("remote: push: {e}"),
        }
    }
}

/// Clear `device`'s registration if it still has `token`.
fn forget_token(dir: &std::path::Path, device: &str, token: &str) {
    let still = registry::devices(dir)
        .ok()
        .and_then(|all| all.into_iter().find(|d| d.id == device))
        .and_then(|d| d.push)
        .is_some_and(|p| p.token == token);
    if still && let Err(e) = registry::set_push(dir, device, None) {
        log::warn!("remote: push: {e}");
    }
}

/// POST `body` (JSON) to `url` with curl: https only, except a loopback
/// `http://` address (tests).
fn post(url: &str, body: &str) -> Result<String, String> {
    let local = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
        .iter()
        .any(|p| url.starts_with(p));
    let protos = if local { "=http" } else { "=https" };
    let mut child = Command::new("curl")
        .args(["-q", "-sS", "--max-time", POST_TIMEOUT_SECS])
        .args(["--proto", protos, "--proto-redir", protos])
        .args(["-H", "Content-Type: application/json"])
        .args(["-H", "Accept: application/json"])
        .args(["--data-binary", "@-", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(body.as_bytes())
            .map_err(|e| format!("curl: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
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

    fn device(id: &str, push: Option<PushReg>) -> Device {
        Device {
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
            token: "ExponentPushToken[abc]".into(),
            long_ms,
            show_command,
        })
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
            token: "ExponentPushToken[abc]".into(),
            title: "A command finished on Mac".into(),
            body: None,
            pane: 12,
        };
        let v: Value = serde_json::from_str(&payload(&[a], "HOSTKEY")).unwrap();
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
}
