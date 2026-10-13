//! `fixtures/noise/xx.json`: a deterministic Noise transcript the
//! TypeScript client replays (`mobile/`). Fixed static and ephemeral private
//! keys (snow's `fixed_ephemeral_key_for_testing_only`), the handshake
//! payloads from `fixtures/handshake/`, then two transport messages each
//! way, alternating gateway → client, client → gateway (the gateway speaks
//! first: `welcome`, `subscribe`, `topology`, `ping`).
//!
//! Message 1 has an empty payload; 2 carries `handshake/gateway.json`; 3
//! carries `handshake/client-pair.json` (each file's bytes without the final
//! newline). Transport payloads are full Noise plaintexts: the chunk flag
//! byte `0x00` and the fixture's JSON. Key fields hold PRIVATE keys (hex);
//! the public keys follow from them.
//!
//! The test regenerates the transcript and fails when the committed file
//! differs. To rewrite it after a deliberate change:
//! `RIFT_UPDATE_FIXTURES=1 cargo test -p rift-remote --test noise_fixture`.

use rift_remote::transport::{self, LAST, NOISE_MAX};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct Message {
    payload_hex: String,
    ciphertext_hex: String,
}

#[derive(Serialize)]
struct Transcript {
    protocol: &'static str,
    prologue_hex: String,
    initiator_static_hex: String,
    initiator_ephemeral_hex: String,
    responder_static_hex: String,
    responder_ephemeral_hex: String,
    messages: Vec<Message>,
    handshake_hash_hex: String,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn fixture(rel: &str) -> Vec<u8> {
    let text = std::fs::read_to_string(fixtures().join(rel)).unwrap();
    text.trim_end_matches('\n').as_bytes().to_vec()
}

/// 32 bytes `start, start+1, …`.
fn key(start: u8) -> [u8; 32] {
    std::array::from_fn(|i| start.wrapping_add(i as u8))
}

fn transcript() -> String {
    let (i_s, i_e, r_s, r_e) = (key(0x01), key(0x21), key(0x41), key(0x61));
    let mut ini = transport::builder()
        .local_private_key(&i_s)
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(&i_e)
        .build_initiator()
        .unwrap();
    let mut res = transport::builder()
        .local_private_key(&r_s)
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(&r_e)
        .build_responder()
        .unwrap();
    let mut messages = Vec::new();
    let (mut wire, mut back) = (vec![0u8; NOISE_MAX], vec![0u8; NOISE_MAX]);
    let mut record = |payload: &[u8], wire: &[u8], back: &[u8]| {
        assert_eq!(back, payload, "the peer decrypts what was sent");
        messages.push(Message {
            payload_hex: hex(payload),
            ciphertext_hex: hex(wire),
        });
    };

    // handshake: → e; ← e, ee, s, es + gateway hello; → s, se + client hello
    let p1: Vec<u8> = Vec::new();
    let n = ini.write_message(&p1, &mut wire).unwrap();
    let m = res.read_message(&wire[..n], &mut back).unwrap();
    record(&p1, &wire[..n], &back[..m]);
    let p2 = fixture("handshake/gateway.json");
    let n = res.write_message(&p2, &mut wire).unwrap();
    let m = ini.read_message(&wire[..n], &mut back).unwrap();
    record(&p2, &wire[..n], &back[..m]);
    assert_eq!(
        ini.get_remote_static().unwrap(),
        transport::public_key(&r_s),
        "what a client pins"
    );
    let p3 = fixture("handshake/client-pair.json");
    let n = ini.write_message(&p3, &mut wire).unwrap();
    let m = res.read_message(&wire[..n], &mut back).unwrap();
    record(&p3, &wire[..n], &back[..m]);
    assert_eq!(ini.get_handshake_hash(), res.get_handshake_hash());
    let hash = hex(ini.get_handshake_hash());
    let mut ini = ini.into_transport_mode().unwrap();
    let mut res = res.into_transport_mode().unwrap();

    for (from_gateway, rel) in [
        (true, "server/welcome.json"),
        (false, "client/subscribe-fresh.json"),
        (true, "server/topology.json"),
        (false, "client/ping.json"),
    ] {
        let mut plain = vec![LAST];
        plain.extend(fixture(rel));
        let (tx, rx) = if from_gateway {
            (&mut res, &mut ini)
        } else {
            (&mut ini, &mut res)
        };
        let n = tx.write_message(&plain, &mut wire).unwrap();
        let m = rx.read_message(&wire[..n], &mut back).unwrap();
        record(&plain, &wire[..n], &back[..m]);
    }

    let t = Transcript {
        protocol: transport::PATTERN,
        prologue_hex: hex(transport::PROLOGUE),
        initiator_static_hex: hex(&i_s),
        initiator_ephemeral_hex: hex(&i_e),
        responder_static_hex: hex(&r_s),
        responder_ephemeral_hex: hex(&r_e),
        messages,
        handshake_hash_hex: hash,
    };
    let mut out = serde_json::to_string_pretty(&t).unwrap();
    out.push('\n');
    out
}

#[test]
fn noise_transcript_matches_the_committed_fixture() {
    let path = fixtures().join("noise/xx.json");
    let now = transcript();
    assert_eq!(now, transcript(), "deterministic");
    if std::env::var_os("RIFT_UPDATE_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &now).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == now,
        "{} differs from the transcript snow produces now; if the change is \
         deliberate, regenerate it (see this file's header) and update the \
         TypeScript side",
        path.display()
    );
    let v: serde_json::Value = serde_json::from_str(&committed).unwrap();
    assert_eq!(v["messages"].as_array().unwrap().len(), 7);
    assert_eq!(v["prologue_hex"], hex(b"rift-remote/2"));
}
