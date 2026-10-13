# Remote protocol (version 2, Phase 1)

The wire format between the Rift remote gateway (crate `rift-remote`, run by
`rift-terminal remote`) and the mobile companion (`mobile/`). Phase 1 of the
[mobile plan](mobile-remote-plan.md): paired devices, end-to-end encryption,
command entry and control keys on the local network. The
[threat model](mobile-threat-model.md) says what it protects.

The Rust types in `crates/rift-remote/src/protocol.rs` and the TypeScript
types in `mobile/src/protocol/types.ts` both decode every file in
`crates/rift-remote/fixtures/`; a change to the format changes those files
first, and both test suites fail until both sides follow. The Noise
interoperability transcript in `fixtures/noise/` is produced by the Rust side
(`snow`) and replayed by the TypeScript side.

## Transport

- One WebSocket per connection, **binary** frames only (a text frame closes
  the connection).
- Every connection runs the Noise handshake
  `Noise_XX_25519_ChaChaPoly_BLAKE2s` with the prologue `rift-remote/2`
  (ASCII). Handshake message 1 (client → gateway), 2 (gateway → client) and
  3 (client → gateway) are one WebSocket frame each. After it, every frame is
  one Noise transport message.
- **Pinning.** The client knows the gateway's static public key from pairing
  and aborts the handshake when message 2 carries another key.
- **Payloads.** Message 2 carries the gateway's `hello` (below); message 3
  carries the client's `hello`.
- **Messages.** After the handshake, each protocol message is a UTF-8 JSON
  object, encrypted in chunks: every Noise plaintext starts with one flag
  byte, `0x00` for the last chunk of a message and `0x01` when more follow,
  then at most 65 000 bytes of the JSON text. A message is decoded once its
  last chunk arrives.
- Every JSON object carries `"t"`, its type. Unknown `"t"` from the client
  gets `error` `bad_request`; the client ignores unknown types (forward
  compatibility).
- The gateway listens on `0.0.0.0:7878` by default (`remote_listen`); the
  pairing link lists every non-loopback IPv4 address it is reachable on.

### Handshake payloads

Gateway (message 2):

```json
{"v":2,"host":"studio-mac","os":"macos","version":"0.1.0"}
```

`host` (here, in `welcome` and in the pairing link's `n`) is the computer's
name as its owner sees it: macOS's ComputerName, Linux's `PRETTY_HOSTNAME`
from `/etc/machine-info`, else the network hostname. The app shows the
latest one it received.

Client (message 3), from a paired device:

```json
{"v":2,"name":"Sam's iPhone","platform":"ios"}
```

and when pairing (first connection with a pairing secret):

```json
{"v":2,"name":"Sam's iPhone","platform":"ios","pair":"<pairing secret, base64url>"}
```

- The gateway accepts the connection when the client's static key is a
  paired device, or when `pair` is a live pairing secret: it then records the
  device (name, platform, static key, the secret's permission) and spends the
  secret. Anything else gets `error` `auth` and the socket closes.
- `v` other than 2 gets `error` `version` and the socket closes.
- The first message after the handshake is the gateway's `welcome`.

## Pairing link

`rift-terminal remote pair` and Settings › Remote access › *Pair a phone*
create a one-time secret (16 random bytes, valid 2 minutes, one use) and
show it as a QR code and as text:

```
rift-remote://pair?a=192.168.1.29:7878,10.0.0.5:7878&k=<gateway static public key, base64url>&s=<secret, base64url>&n=studio-mac&p=operate
```

- `a`: addresses to try in order; `k`: the key the client pins; `s`: the
  secret; `n`: the host name to show; `p`: `operate` or `view`.
- The app opens this link from its scanner, from a paste, or as a deep link
  (the system camera scanning the QR opens it).

## Devices and permissions

- `operate`: everything below. `view`: no `run`, no `keys` (they get
  `error` `forbidden`).
- The desktop lists, renames nothing, and revokes devices (Settings ›
  Remote access, or `rift-terminal remote devices` / `revoke ID`). A revoked
  device's open connections close within 1 second, and its key is refused
  from then on.
- A client may remove itself with `forget`.

## Limits

| Limit | Value |
|---|---|
| Client message | 4 KiB of JSON; larger closes the connection |
| Gateway message | 1 MiB of JSON (in ≤ 65 000-byte chunks) |
| Concurrent connections | 8; the ninth is refused with `error` `busy` after the handshake |
| Handshake deadline | 5 s after the socket opens |
| Failed handshakes | 10 per minute per remote address; then refused for a minute |
| Idle | the gateway pings every 15 s and drops a client silent for 45 s |
| Blocks per `snapshot` / `older` | 30, newest last |
| Output per block | the last 500 lines and at most 64 KiB; `dropped` counts the lines cut from the head that are still in the window's scrollback (lines the scrollback limit already discarded are not counted) |
| Replay ring | the last 256 `update`s per pane |
| Poll interval | 100 ms for subscribed panes, 1 s for the topology |
| `run` | 4 per second per connection; the command at most 2 KiB, one line |
| `keys` | 20 keys per message, 20 messages per second per connection |

## Client → gateway

```json
{"t":"subscribe","pane":12,"after":null}
{"t":"subscribe","pane":12,"after":41}
{"t":"older","pane":12,"before":87}
{"t":"unsubscribe"}
{"t":"run","id":"r1","pane":12,"command":"cargo fmt --all"}
{"t":"keys","id":"k1","pane":12,"keys":["ctrl+c"]}
{"t":"forget"}
{"t":"ping","n":3}
{"t":"push","token":"ExponentPushToken[…]","long_ms":30000,"show_command":false}
{"t":"push","token":null,"long_ms":null,"show_command":false}
{"t":"notify","pane":12}
```

- `subscribe` follows one pane (a new `subscribe` replaces the old one).
  `after` is the last `seq` the client applied for that pane: when the ring
  still holds every `update` after it, the gateway replays them; otherwise (or
  with `after: null`) it sends a fresh `snapshot`.
- `older` asks for up to 30 blocks older than block id `before`.
- `run` submits a command line to a pane that is at its prompt, exactly as if
  it were typed in the pane's editor and Enter pressed: it becomes a block,
  goes into history, and the desktop shows a toast naming the device. A pane
  that is running something answers `ack` with `error` `busy`; use `keys`
  there. The command may not contain a newline or a control character.
- `keys` sends keys to the pane as if pressed on the desktop (raw to the
  program when one is running, to the editor at a prompt). Names:
  `ctrl+a` … `ctrl+z`, `esc`, `tab`, `shift+tab`, `enter`, `backspace`,
  `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`.
- `id` is chosen by the client (at most 64 characters) and returned in `ack`.
- `forget` removes this device from the desktop's paired devices and closes
  the connection.
- `ping` is answered with `pong` carrying the same `n`.
- `push` and `notify`: alerts while the app is closed (see § Push). No
  reply; a malformed one gets `error` `bad_request`.

## Gateway → client

```json
{"t":"welcome","v":2,"host":"studio-mac","os":"macos","version":"0.1.0","device":"d3f1","permission":"operate"}
{"t":"topology","windows":[{"pid":4242,"tabs":[{"index":0,"title":"rift","active":true,"panes":[{"id":12,"active":true,"cwd":"/Users/me/rift","job":null,"last_command":"cargo test","last_exit":0}]}]}]}
{"t":"snapshot","pane":12,"seq":41,"blocks":[Block],"has_older":true}
{"t":"update","pane":12,"seq":42,"blocks":[Block],"removed":[80]}
{"t":"older","pane":12,"blocks":[Block],"has_older":false}
{"t":"ack","id":"r1","ok":true}
{"t":"ack","id":"r2","ok":false,"error":"busy","message":"pane 12 is running a command"}
{"t":"done","pane":12,"block":87,"command":"cargo test","exit":0,"duration_ms":260500}
{"t":"pong","n":3}
{"t":"error","code":"auth","message":"this device is not paired"}
```

- `welcome` follows the handshake, then `topology`, which is sent again
  whenever it changes.
- `seq` is per pane and per gateway run: `update` N+1 follows `update` (or
  `snapshot`) N. A client that sees a gap re-subscribes with its last `seq`.
- `update.blocks` are whole blocks that are new or changed since the
  previous `seq` (upsert by `id`); `removed` lists ids gone from the pane
  (`clear`, scrollback trim), including ids the client received through
  `older`. A running block's clock alone is not a change: `duration_ms` is
  current whenever the block is sent for another reason.
- No gateway message exceeds 1 MiB: a larger `update` is split into several,
  each with its own `seq`; a `snapshot` or `older` page that would not fit
  leaves out its oldest blocks and reports `has_older: true`.
- `done` is sent to every connection when any pane's command finishes (for
  notifications), whether or not the client follows that pane.
- A pane that closes is reported by `topology` and by `error` `no_pane` for
  its subscription.
- Error codes: `auth`, `version`, `bad_request`, `busy`, `forbidden`,
  `no_pane`, `too_large`, `rate`. `ack.error` uses the same codes.

### Block

```json
{
  "id": 87,
  "command": "cargo test --workspace",
  "state": "running",
  "exit": null,
  "duration_ms": 252000,
  "cwd": "/Users/me/rift",
  "branch": "main",
  "output": "test block::tests::osc133_idempotent ... ok\n",
  "dropped": 0,
  "restored": false
}
```

- `state`: `running` or `done` (a prompt being typed is not sent).
- `exit`: the exit status once `done` (may stay `null` when the shell did
  not report one).
- `duration_ms`: elapsed time while `running`, final duration when `done`.
- `output`: plain text, no escape sequences; soft-wrapped rows are joined.
- `restored`: replayed from a saved transcript of an earlier session.

## Relay (Phase 2)

For use outside the local network, a gateway can also be reached through a
relay: `rift-terminal relay` (or the `packaging/relay.Dockerfile` image),
which anyone can host. The relay only splices WebSocket connections; it
never holds a key and only ever sees Noise ciphertext (and the routing id).
It listens on plain `ws://`; put it behind a TLS reverse proxy and give
gateways and phones its `wss://` URL.

- **Routing id**: base64url of the first 16 bytes of SHA-256 over the
  gateway's static public key. It names the gateway at the relay and is no
  secret.
- **Gateway registration**: the gateway (config `remote_relay = "wss://…"`)
  keeps one control connection open to `<relay>/gateway` and sends
  `{"t":"register","id":"<routing id>","key":"<static public key, base64url>","secret":"<32 random bytes, base64url>"}`.
  The relay closes it unless `id` is `key`'s routing id, then checks that
  the gateway holds the key: it sends `{"t":"prove","m":"<base64url>"}`,
  message 1 of a `Noise_NK_25519_ChaChaPoly_BLAKE2s` handshake (prologue
  `rift-relay/1`, empty payload) to `key`, and the gateway answers
  `{"t":"proof","m":"<base64url>"}` with message 2; a proof that does not
  read closes the connection. So only the key's holder can claim its id:
  a proved registration replaces any earlier control connection and secret
  of that id (a reconnect or a restart). When the relay holds 1000 ids it
  forgets the one offline the longest (without pairs) to make room, and
  answers `{"t":"error","code":"full"}` only when every id is online or
  busy. The HTTP upgrade and the whole registration must finish within
  10 s of the socket opening. The relay pings control connections every 20 s and drops one silent for
  60 s; the gateway pings every 15 s, redials after 45 s of silence, and
  backs off 1 s → 30 s between attempts. A path in the relay URL is kept
  (`wss://host/rift` → `/rift/gateway`). The secret is kept in
  `<data dir>/remote/relay.json` (0600) so restarts keep the id. A
  successful registration is answered with `{"t":"registered"}`.
- **Phone connection**: the phone opens `<relay>/connect?id=<routing id>`.
  The relay sends `{"t":"open","conn":"<random 16-byte id, base64url>"}` on
  the gateway's control connection; the gateway opens
  `<relay>/accept?id=<routing id>&conn=<conn>` with the header
  `Authorization: Bearer <secret>` (never in the URL, which proxies log) and
  the relay then forwards every binary frame between the two sockets
  unchanged, in both directions. The gateway treats that socket like a
  direct one (same Noise handshake and limits), but relay sockets have
  their own budget of 8 sessions and 16 sockets, so `open`s, which anyone
  who knows the routing id can cause, never refuse a phone on the LAN.
  A phone whose gateway does not accept within 10 seconds is closed. Text
  frames on a spliced pair close both. Through the relay, WebSocket pongs
  come from the relay itself, so the gateway's 45 s idle limit counts only
  the phone's Noise messages (the app pings in them every 15 s).
- **Relay limits**: 32 pairs per id (waiting and spliced), 1 MiB per frame,
  60 s idle per pair (Noise pings keep live ones open), 30 `/connect` per
  minute per remote address, 64 open sockets per address, 1000 registered
  ids, 4096 sockets, 10 s for an upgrade in total (a request sent a byte at
  a time is cut there). Addresses count per IPv4 address or IPv6 /64.
  Control messages are text frames of at most 1 KiB. Refusals are HTTP
  statuses at the upgrade: `/connect` 400 (bad id), 404 (gateway offline),
  429 (rate), 503 (pairs full); `/accept` 400 (no secret), 403 (wrong secret
  or id), 404 (unknown conn). Behind a reverse proxy, `--trust-forwarded`
  takes the client address from the last `X-Forwarded-For` entry (the one
  the proxy wrote; when it is not an address, the peer's) and does not
  count sockets per address.
- **Pairing link**: `r=<relay URL>` is added when the gateway has a relay;
  when pairing, the phone tries the `a` addresses first (1.5 s each), then
  the relay. A paired host's rounds start at the path that last reached
  `welcome` and go on through the rest in order (addresses, then the relay,
  wrapping around); a network change starts at the first address. A LAN
  address that answers with another key than the pinned one is skipped; a
  mismatch on the relay, or on every path of a round, stops the client
  (`key_mismatch`) until the user retries or the network changes.
- **What the relay learns**: which routing ids are online, when phones
  connect, from which addresses, and how many bytes flow. Never content,
  keys or device identities.

## Push (alerts while the app is closed)

The app can only post notifications while it runs, and Android 15+ and iOS
stop a backgrounded app's network within seconds. So a device can hand the
gateway an Expo push token, and the gateway alerts through the Expo push
service (which hands on to Apple's APNs or Google's FCM) by itself.

- `push` stores (or with `token: null` removes) this device's registration
  in `devices.json` (`"push":{"token","long_ms","show_command"}`). The token
  must look like `ExponentPushToken[…]` / `ExpoPushToken[…]` (URL-safe
  inside, at most 200 characters); `long_ms` is at most a day.
- `notify` asks for one alert when the next command in `pane` finishes,
  whatever its length ("Notify me"); at most 32 panes per device, each
  forgotten after 24 h.
- For every `done` the gateway alerts each device with a token that asked
  about that pane (the wait is used up) or whose `long_ms` the command
  reached, at most 12 per device per minute. While a device has a token the
  gateway watches every pane even with nobody connected.
- The request is a POST of JSON to `https://exp.host/--/api/v2/push/send`
  (with `curl`, https only): `title` `A command finished on <host>` or
  `A command failed on <host> (exit N)`, `body` the command line only when
  `show_command`, `data` `{"host":"<gateway key>","pane":N}` (a tap opens
  the pane), `priority` `high`, `sound` `default`. Output never goes out.
- A token the service answers with `DeviceNotRegistered` is removed.
- The app then posts no notification of its own for that computer (one alert
  per command), and drops an alert about the pane on screen.

## Connection history and permissions (Phase 2)

- `<data dir>/remote/history.jsonl` (0600): one line per closed
  connection, `{"device","name","via":"lan"|"relay","address","start","end","reason"}`
  (times RFC 3339, whole seconds), 500 kept. Failed handshakes are
  recorded with `device` and `name` null; when the file is full they are
  dropped first (oldest first), so a flood of refused attempts never pushes
  out a paired device's connections. `reason` is one of `closed`,
  `error`, `idle`, `too_slow`, `too_large`, `bad_request`, `revoked`,
  `permission`, `forgotten`, `stopped`, or for a failed handshake
  `handshake`, `version`, `auth`, `busy`.
- Through the relay the gateway cannot see the phone's address: `address`
  is the relay's `host:port`, and the per-address failed-handshake limit
  does not apply there (the relay's per-address `/connect` limit does).
  Someone who knows a routing id can occupy that gateway's relay sockets
  (within the relay's per-address limits) and slow relay phones down; LAN
  phones are unaffected (separate budget).
- A device's permission can be changed on the desktop (Settings › Remote
  access, or `rift-terminal remote permission ID operate|view`); open
  connections of that device close within a second so the next handshake
  carries the new permission. The gateway reads `devices.json` at every
  handshake and every `run` / `keys`, so a lowered permission applies at
  once, even before the open connections close.

## Desktop side

| Path (under `<data dir>/remote/`, directory 0700, files 0600) | Holds |
|---|---|
| `gateway.key` | The gateway's static X25519 private key (32 bytes), created once |
| `devices.json` | `[{"id","name","platform","key","permission","paired_at","last_seen","push"?}]` (`key` base64url, times RFC 3339, `push` see § Push) |
| `pairing.json` | Live pairing secrets: `[{"hash","permission","expires_at"}]` (SHA-256 of the secret, never the secret) |
| `status.json` | Written by the running gateway: `{"pid","addresses":[...],"clients":[{"device","name","since","via"}],"relay"?:{"url","state","error"?}}` (`relay` only while one is configured); removed when it stops |
| `relay.json` | `{"secret":"<32 bytes, base64url>"}`: the gateway's `/accept` secret at the relay |
| `history.jsonl` | Connection history (see above) |
| `gateway.lock` | Held (flock) by the running gateway; one gateway per user |

- `remote_access = false` (config, default) keeps the gateway stopped. When
  it is `true`, every Rift window makes sure a gateway runs
  (`rift-terminal remote --managed`, detached); the gateway exits within 2
  seconds after `remote_access` becomes `false`, and 30 seconds after the
  last Rift window is gone.
- A gateway counts as running only while `gateway.lock` is held; a stale
  `status.json` is ignored. A window starts one at most every 5 seconds.
- The gateway reads windows through their control sockets: `status` and
  `blocks` (read-only; `"meta": true` leaves the output out, which the
  `done` probe uses), and `remote-input`
  (`{"cmd":"remote-input","pane":12,"device":"Sam's iPhone","command":"…"}`
  or `{…,"keys":["ctrl+c"]}`), which a window accepts only while
  `remote_access` is `true`, independent of `control_api`. Its errors:
  `remote access is off`, `no such pane`, `busy` (the pane is running
  something, or a command was submitted and has not started yet),
  `bad request: …`.
- Every window shows a tab-bar chip while at least one device is connected
  (from `status.json`); clicking it opens Settings › Remote access. After
  every `status.json` change the gateway sends `{"cmd":"remote-status"}`
  (best effort, 500 ms timeout, same level as `status`) to each window, which
  re-reads the file and redraws, so an idle window updates within a second.
