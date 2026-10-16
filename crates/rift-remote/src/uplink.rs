//! The gateway's side of a relay (`docs/remote-protocol.md` § Relay): one
//! control connection to `<relay>/gateway` registered with the routing id,
//! the static public key (proved with [`relay::prove`]) and the secret from
//! `relay.json`, kept open (pings, reconnect with backoff and jitter), and
//! for every `open` a socket dialed at `<relay>/accept` that runs the same
//! [`connection`] as a direct one. Relay sockets have their own budget
//! (`relay_connections`), so a flood of `open`s never refuses LAN phones.
//!
//! The state shows in `status.json` (`relay`): `connecting` while dialing
//! or registering, `connected` once registered, `error` with the reason
//! after a failure (until the next try).

use crate::Source;
use crate::protocol::{RelayError, RelayMsg};
use crate::registry::{self, RelayState, RelayStatus, Via};
use crate::relay::{self, MAX_CONTROL, RelaySocket, RelayUrl};
use crate::server::{Peer, READ_SLICE, Shared, admit, client_ws_config, connection, write_status};
use crate::transport::{self, Socket};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tungstenite::Message;
use tungstenite::protocol::WebSocketConfig;

/// DNS, TCP, TLS and the upgrade of the control connection.
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
/// The relay answers `register` this soon.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);

/// The relay connection's thread: follows [`Shared::relay`] until the
/// gateway stops.
pub(crate) fn run<S: Source>(shared: Arc<Shared<S>>) {
    let (min, max) = shared.config.relay_backoff;
    let mut backoff = min;
    let mut warned: Option<String> = None;
    // a request left by a window while no gateway ran is stale
    registry::take_relay_retry(&shared.config.dir);
    while !shared.stop.load(Ordering::Acquire) {
        let (url, generation) = {
            let link = shared.relay.lock().unwrap();
            (link.url.clone(), link.generation)
        };
        let Some(url) = url else {
            backoff = min;
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        set_state(&shared, generation, RelayState::Connecting, None);
        let result = RelayUrl::parse(&url).and_then(|relay| {
            if !relay.tls && warned.as_deref() != Some(url.as_str()) {
                log::warn!(
                    "remote: relay {url} is plain ws://: the relay's operator and the network \
                     see routing ids and timing (content stays Noise-encrypted); use wss://"
                );
                warned = Some(url.clone());
            }
            control(&shared, &relay, generation, &mut backoff)
        });
        if let Err(e) = &result {
            log::warn!("remote: relay {url}: {e}");
            set_state(&shared, generation, RelayState::Error, Some(e.clone()));
        }
        let changed = || {
            shared.stop.load(Ordering::Acquire)
                || shared.relay.lock().unwrap().generation != generation
        };
        if changed() {
            backoff = min;
            continue;
        }
        let delay = jitter(backoff);
        backoff = (backoff * 2).min(max);
        let until = Instant::now() + delay;
        if result.is_err() {
            set_retry(&shared, generation, delay);
        }
        // "Try again" in Settings ends the wait at once
        while Instant::now() < until
            && !changed()
            && !registry::take_relay_retry(&shared.config.dir)
        {
            std::thread::sleep(Duration::from_millis(50).min(delay));
        }
    }
}

/// `base` plus up to half of it again, at random.
fn jitter(base: Duration) -> Duration {
    let r = transport::random_bytes::<2>()
        .map(u16::from_le_bytes)
        .unwrap_or(0);
    base + base.mul_f64(f64::from(r) / f64::from(u16::MAX) / 2.0)
}

/// Record the state of `generation`'s connection (ignored once the setting
/// changed) and rewrite `status.json` when it changed.
fn set_state<S: Source>(
    shared: &Shared<S>,
    generation: u64,
    state: RelayState,
    error: Option<String>,
) {
    {
        let mut link = shared.relay.lock().unwrap();
        if link.generation != generation {
            return;
        }
        let Some(url) = link.url.clone() else {
            return;
        };
        let next = RelayStatus {
            url,
            state,
            error,
            latency_ms: None,
            retry_at_ms: None,
        };
        if link.status.as_ref() == Some(&next) {
            return;
        }
        link.status = Some(next);
    }
    write_status(shared);
}

/// Record when the next try after a failure starts (`status.json`).
fn set_retry<S: Source>(shared: &Shared<S>, generation: u64, delay: Duration) {
    let at = std::time::SystemTime::now() + delay;
    let ms = at
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    update_status(shared, generation, |s| {
        if s.state == RelayState::Error {
            s.retry_at_ms = Some(ms);
        }
    });
}

/// Record the round trip of the last ping while connected (`status.json`).
fn set_latency<S: Source>(shared: &Shared<S>, generation: u64, rtt: Duration) {
    // whole ms, rounded up: a loopback relay is 1 ms, never "0 ms"
    let ms = rtt
        .as_micros()
        .div_ceil(1000)
        .clamp(1, u128::from(u32::MAX)) as u32;
    update_status(shared, generation, |s| {
        if s.state == RelayState::Connected {
            s.latency_ms = Some(ms);
        }
    });
}

/// Change `generation`'s status in place and rewrite `status.json` when it
/// differs.
fn update_status<S: Source>(
    shared: &Shared<S>,
    generation: u64,
    change: impl FnOnce(&mut RelayStatus),
) {
    {
        let mut link = shared.relay.lock().unwrap();
        if link.generation != generation {
            return;
        }
        let Some(status) = link.status.as_mut() else {
            return;
        };
        let before = status.clone();
        change(status);
        if *status == before {
            return;
        }
    }
    write_status(shared);
}

/// A socket error as the relay state shows it.
fn lost(e: &tungstenite::Error) -> String {
    use tungstenite::Error as E;
    match e {
        E::ConnectionClosed | E::AlreadyClosed | E::Io(_) | E::Protocol(_) => {
            "lost the connection to the relay".into()
        }
        e => e.to_string(),
    }
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

/// One control connection: dial, register, then serve `open`s until it
/// drops (`Err`), or the gateway stops or the setting changes (`Ok`).
/// `backoff` starts over once registered.
fn control<S: Source>(
    shared: &Arc<Shared<S>>,
    relay: &RelayUrl,
    generation: u64,
    backoff: &mut Duration,
) -> Result<(), String> {
    let dir = &shared.config.dir;
    let id = registry::routing_id(&transport::public_key(&shared.key));
    let secret = B64.encode(registry::relay_secret(dir).map_err(|e| format!("relay.json: {e}"))?);
    let ws_config = WebSocketConfig::default()
        .max_frame_size(Some(MAX_CONTROL))
        .max_message_size(Some(MAX_CONTROL));
    let mut ws = relay::dial(relay, "/gateway", None, ws_config, DIAL_TIMEOUT)?;
    let register = RelayMsg::Register {
        id: id.clone(),
        key: registry::encode_key(&transport::public_key(&shared.key)),
        secret: secret.clone(),
    };
    ws.send(Message::text(
        serde_json::to_string(&register).unwrap_or_default(),
    ))
    .map_err(|e| lost(&e))?;
    let deadline = Instant::now() + REGISTER_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err("the relay did not answer the registration".into());
        }
        match ws.read() {
            Ok(Message::Text(t)) => match serde_json::from_str::<RelayMsg>(t.as_str()) {
                Ok(RelayMsg::Registered) => break,
                Ok(RelayMsg::Prove { m }) => {
                    let proof = B64
                        .decode(m.as_bytes())
                        .ok()
                        .and_then(|m| relay::prove(&shared.key, &m).ok())
                        .ok_or("the relay sent a malformed key challenge")?;
                    let proof = RelayMsg::Proof {
                        m: B64.encode(proof),
                    };
                    ws.send(Message::text(
                        serde_json::to_string(&proof).unwrap_or_default(),
                    ))
                    .map_err(|e| lost(&e))?;
                }
                Ok(RelayMsg::Error {
                    code: RelayError::Full,
                }) => return Err("the relay is full".into()),
                _ => {}
            },
            Ok(Message::Close(_)) => return Err("the relay closed the connection".into()),
            Ok(_) => {}
            Err(e) if would_block(&e) => {}
            Err(e) => return Err(lost(&e)),
        }
    }
    *backoff = shared.config.relay_backoff.0;
    log::info!("remote: registered at relay {} as {id}", relay.authority());
    set_state(shared, generation, RelayState::Connected, None);
    ws.get_ref()
        .set_read_timeout(Some(READ_SLICE * 4))
        .map_err(|e| e.to_string())?;
    serve_control(shared, relay, generation, &mut ws, &id, &secret)
}

fn serve_control<S: Source>(
    shared: &Arc<Shared<S>>,
    relay: &RelayUrl,
    generation: u64,
    ws: &mut RelaySocket,
    id: &str,
    secret: &str,
) -> Result<(), String> {
    let cfg = &shared.config;
    let mut heard = Instant::now();
    // the first ping goes out at once: Settings shows its round trip
    let mut pinged = Instant::now()
        .checked_sub(cfg.ping_interval)
        .unwrap_or_else(Instant::now);
    // when the ping still unanswered went out
    let mut waiting: Option<Instant> = None;
    loop {
        if shared.stop.load(Ordering::Acquire)
            || shared.relay.lock().unwrap().generation != generation
        {
            let _ = ws.close(None);
            let _ = ws.flush();
            return Ok(());
        }
        if heard.elapsed() >= cfg.idle_timeout {
            return Err("the relay stopped answering".into());
        }
        if pinged.elapsed() >= cfg.ping_interval {
            pinged = Instant::now();
            waiting = Some(pinged);
            ws.send(Message::Ping(Vec::new().into()))
                .map_err(|e| lost(&e))?;
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                heard = Instant::now();
                if let Ok(RelayMsg::Open { conn }) = serde_json::from_str::<RelayMsg>(t.as_str()) {
                    accept(shared, relay, id, secret, conn);
                }
            }
            Ok(Message::Close(_)) => return Err("the relay closed the connection".into()),
            Ok(Message::Pong(_)) => {
                heard = Instant::now();
                if let Some(sent) = waiting.take() {
                    set_latency(shared, generation, sent.elapsed());
                }
            }
            Ok(_) => heard = Instant::now(),
            Err(e) if would_block(&e) => {}
            Err(e) => return Err(lost(&e)),
        }
    }
}

/// `conn` as the relay sends it: base64url, as it goes into a URL query.
fn valid_conn(conn: &str) -> bool {
    !conn.is_empty()
        && conn.len() <= 32
        && conn
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Dial `/accept` for `conn` on a thread of its own and serve it like a
/// direct connection (the relay sockets' own count, handshake deadline
/// from now).
fn accept<S: Source>(
    shared: &Arc<Shared<S>>,
    relay: &RelayUrl,
    id: &str,
    secret: &str,
    conn: String,
) {
    if !valid_conn(&conn) {
        return;
    }
    let opened = Instant::now();
    let path = format!("/accept?id={id}&conn={conn}");
    let secret = secret.to_string();
    let (shared, relay) = (shared.clone(), relay.clone());
    let spawned = std::thread::Builder::new()
        .name("remote-client".into())
        .spawn(move || {
            let Some((busy, _guard)) = admit(&shared.relay_connections) else {
                return;
            };
            let timeout = shared
                .config
                .handshake_deadline
                .saturating_sub(opened.elapsed())
                .max(Duration::from_millis(100));
            let ws = match relay::dial(&relay, &path, Some(&secret), client_ws_config(), timeout) {
                Ok(ws) => ws,
                Err(e) => {
                    log::info!("remote: relay accept {conn}: {e}");
                    return;
                }
            };
            let peer = Peer {
                via: Via::Relay,
                address: relay.authority(),
                ip: None,
            };
            connection(&shared, ws, peer, opened, busy);
        });
    if let Err(e) = spawned {
        log::warn!("remote: cannot start a client thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_adds_at_most_half() {
        let base = Duration::from_secs(4);
        for _ in 0..50 {
            let d = jitter(base);
            assert!(d >= base && d <= base + base / 2, "{d:?}");
        }
    }

    #[test]
    fn conn_ids_are_url_safe() {
        assert!(valid_conn("EBESExQVFhcYGRobHB0eHw"));
        for bad in ["", "a&secret=x", "a b", "a/b", "a%20", &"a".repeat(33)] {
            assert!(!valid_conn(bad), "{bad}");
        }
    }
}
