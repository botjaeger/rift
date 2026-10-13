# Rift Mobile Remote Companion

## Summary

Build an Expo-based companion app for iOS and Android that lets users securely
access terminal sessions already running in Rift on their desktop.

The mobile app is a remote client, not a second terminal emulator. Rift on the
desktop continues to own the shell process, PTY, terminal parser, command blocks,
scrollback, and session state. Mobile receives a synchronized, touch-friendly
view and sends narrowly scoped user actions back to the desktop.

This positioning is important: Rift Mobile should be “your Rift sessions in
your pocket,” not another generic SSH client.

## Status

Phases 0–2 are built: the gateway (crate `rift-remote`, started from
Settings › Remote access), the relay (`rift-terminal relay`, `rift-relay`,
`packaging/relay.Dockerfile`), the companion in
[`mobile/`](../mobile/README.md), the wire format in
[remote-protocol.md](remote-protocol.md) and the
[threat model](mobile-threat-model.md). Decisions taken: Noise XX
(`Noise_XX_25519_ChaChaPoly_BLAKE2s`) over WebSocket; the desktop key pinned
through a one-time QR pairing link (no mDNS yet); per-device `operate` /
`view` permissions, changeable on the desktop; finish alerts through Expo push
while the app is closed (never output; the command line only when the user
asks); a
self-hostable opaque relay (rendezvous and splice, routing ids claimed by
proof of the gateway key) for outside the local network, with private networks such as
Tailscale working through the address list. Phase 3 (raw terminal mode,
keyboard mappings) is next.

## Why Expo

Expo with React Native and TypeScript is the preferred starting point because
the intended client is primarily a structured application UI: session lists,
command blocks, command input, pairing, settings, and notifications. It does not
need to reproduce Rift's native GPU terminal renderer.

Expo provides:

- One codebase for iOS and Android, with a useful web target for development.
- Fast product iteration and over-the-air JavaScript updates.
- Good building blocks for QR pairing, deep links, secure storage, biometrics,
  notifications, and WebSocket communication.
- A natural fit for Rift's block-oriented interface.
- Development builds when a capability is unavailable in Expo Go.

Start with Expo development builds rather than treating Expo Go compatibility as
a requirement. This leaves room for audited native cryptography, local-network
discovery, keyboard handling, and background behavior.

## System boundary

```text
┌──────────────────────────────┐
│ Expo companion              │
│                              │
│ Session and pane browser     │
│ Structured block list        │
│ Command composer             │
│ Touch and control-key input  │
└──────────────┬───────────────┘
               │ encrypted, authenticated protocol
               │ WebSocket initially
┌──────────────▼───────────────┐
│ Rift desktop remote gateway│
│                              │
│ Pairing and authorization    │
│ Protocol and event adapter   │
│ Rate and resource limits     │
└──────────────┬───────────────┘
               │ internal typed events
┌──────────────▼───────────────┐
│ Existing Rift application  │
│ tabs → panes → PTYs → shells │
└──────────────────────────────┘
```

The remote gateway should adapt existing application state into a versioned
protocol. It must not give clients direct PTY, shell, filesystem, or arbitrary
desktop API access.

## MVP scope

The first useful release should allow a user to:

1. Enable remote access explicitly in Rift desktop.
2. Pair one phone by scanning a short-lived QR code.
3. See connected Rift hosts and their tabs and panes.
4. Open a pane and view recent command blocks and live output.
5. Compose and run a command after clear confirmation.
6. Send common terminal controls such as Ctrl-C, Escape, Tab, and arrow keys.
7. Reconnect and recover the latest state after a network interruption.
8. Receive an optional notification when a command finishes or fails.
9. Revoke a paired device from the desktop.

Defer full terminal-grid fidelity, file browsing, port forwarding, collaborative
sessions, voice input, and unrestricted background execution until the core
security and synchronization model is proven.

## Mobile experience

Prefer structured blocks over a pixel-equivalent terminal canvas:

- A host screen shows reachable and previously paired desktops.
- A session screen shows tabs, panes, current directory, shell status, and Git
  context when available.
- A pane screen presents virtualized command blocks with command, output, exit
  status, timestamps, collapse state, and bookmarks.
- A persistent composer provides command input, history, and a compact
  row of terminal control keys.
- A raw terminal view can be added later for full-screen TUIs and alternate-screen
  programs.

Use a strongly virtualized list and batch high-frequency output updates. The
desktop should parse terminal output once and send structured changes where
possible instead of continuously streaming rendered frames.

## Protocol direction

Use a small, versioned message protocol shared by Rust and TypeScript. Generate
TypeScript definitions from the Rust schema, or validate both implementations
against a language-neutral schema. Do not maintain unrelated handwritten types.

Initial message families:

- Connection: protocol negotiation, authentication, heartbeat, and errors.
- Topology: hosts, sessions, tabs, panes, focus, and lifecycle changes.
- Snapshot: bounded initial state and resumable sequence position.
- Output: block creation, output append, metadata changes, and terminal-mode hints.
- Input: command submission, text input, key events, resize, and interrupt.
- Presence: connection state and active mobile viewer.
- Notification: command completion and failure events.

Every event stream should carry monotonic sequence numbers. On reconnect, the
client requests changes after its last acknowledged sequence; if those changes
are no longer retained, the desktop sends a fresh bounded snapshot.

Apply backpressure and explicit payload limits from the beginning. Large output
must be chunked, and the mobile client must be able to request older scrollback
rather than receiving the complete history automatically.

## Security model

Remote terminal access is equivalent to remote code execution on the user's
computer, so security is a product requirement rather than a later hardening task.

- Remote access is disabled by default and visibly indicated while enabled.
- Pairing requires physical access or explicit approval on the desktop.
- QR payloads contain only short-lived, single-use pairing material.
- Each device receives a distinct identity and revocable credential.
- Transport is mutually authenticated and end-to-end encrypted.
- Long-lived credentials are stored using platform secure storage.
- Sensitive local actions may require device biometrics after reconnect.
- Desktop users can inspect and revoke paired devices and active connections.
- Commands always target an explicit host, session, and pane.
- New or destructive command submissions remain visible on the desktop.
- Logs record connection and authorization events but never credentials or raw
  secrets typed into the terminal.
- Protocol parsing, decompression, scrollback, message rate, and client count all
  have conservative resource limits.

Do not expose the desktop listener directly to the public internet for the MVP.
Start with local-network connectivity. A later remote relay should forward opaque
encrypted traffic and must not hold session decryption keys.

## Suggested client stack

- Expo and React Native with TypeScript
- Expo Router for navigation
- Development builds and EAS Build
- WebSocket transport with reconnect and heartbeat handling
- `expo-secure-store` for device credentials
- Expo Notifications for command completion alerts
- A virtualized list such as FlashList for block output
- A generated or schema-validated shared protocol package

Choose cryptographic primitives and libraries only after a focused security
design review. Application-level encryption should use a well-reviewed protocol
or library rather than custom cryptography assembled from low-level primitives.

## Desktop work

The desktop implementation will need:

- A remote-access service with an independent lifecycle from individual panes.
- Read-only projections of tabs, panes, blocks, terminal modes, and connection
  status.
- A controlled command and key-input API that reuses existing application actions.
- Incremental event publication with bounded replay buffers.
- Pairing, device identity, permission, and revocation storage.
- Settings and visible indicators for remote access and active clients.
- Local-network listener discovery and safe bind-address defaults.
- Notification event production without depending on the mobile connection.

Keep networking and protocol logic outside the main application coordinator. A
dedicated crate, tentatively `rift-remote`, can own protocol types, framing,
authentication, synchronization, and transport adapters while `rift-terminal`
bridges those messages to application actions.

## Delivery phases

### Phase 0: protocol spike

- Define the trust model and write a short threat model.
- Prove desktop-to-Expo communication on a local network.
- Stream a bounded, read-only pane snapshot and incremental output.
- Measure update frequency, memory use, and long-output behavior on a real phone.

Exit criterion: a phone can reconnect to a running pane and display live output
without unbounded buffering or UI stalls.

### Phase 1: private MVP

- QR pairing and revocation.
- Host, tab, and pane navigation.
- Structured blocks, live output, command entry, and common control keys.
- Foreground reconnect and basic completion notifications.
- Automated protocol, authorization, reconnect, and malformed-input tests.

Exit criterion: trusted testers can safely operate desktop sessions on the same
network during normal development work.

### Phase 2: remote connectivity

- Evaluate an opaque relay or a supported private-network integration.
- Add offline/reconnect behavior suitable for changing networks.
- Add biometric unlock, richer permission controls, and connection history.
- Complete mobile accessibility, battery, and adverse-network testing.

Exit criterion: remote access works reliably outside the local network without
the relay being able to read terminal content.

### Phase 3: broader terminal support

- Raw terminal mode for full-screen TUIs.
- Better external-keyboard mappings and configurable control rows.
- Structured command actions.
- Selective scrollback retrieval and richer notification policies.

## Testing strategy

- Unit-test serialization, authorization decisions, sequence handling, and limits.
- Maintain protocol compatibility fixtures shared between Rust and TypeScript.
- Use property and fuzz testing on all desktop protocol decoders.
- Integration-test pairing, revocation, reconnect, replay, and snapshot fallback.
- Test slow consumers, reordered or duplicated events, huge output, and malformed
  frames.
- Exercise background/foreground transitions and network changes on physical iOS
  and Android devices.
- Include security review before any internet-accessible release.

## Success measures

- Pairing completes in under one minute without manually copying credentials.
- Reconnection on the same network normally restores state within a few seconds.
- Large command output remains responsive and does not cause unbounded memory use.
- Users can identify the active host, pane, and command target before submitting.
- Revocation prevents subsequent access without requiring a Rift restart.
- The desktop remains fully usable when mobile networking is disabled or fails.

## Open decisions

- Whether the first transport is direct TLS, a Noise-based channel, or another
  established mutually authenticated protocol.
- Whether discovery uses mDNS, a QR-encoded address, or both.
- How much structured terminal state is stable enough to expose as public protocol.
- Whether command submission and arbitrary key input use separate permission levels.
- Whether remote internet access uses a Rift-operated opaque relay or integrates
  with an existing private-network product.
- Which raw-terminal renderer is appropriate if full-screen TUI support becomes a
  priority.

## Immediate next steps

1. Write the threat model and choose the pairing/authentication protocol.
2. Define the smallest versioned protocol for host topology and read-only blocks.
3. Create an Expo development-build spike with a virtualized block list.
4. Add a desktop prototype service behind an experimental feature flag.
5. Test snapshot, live streaming, disconnect, and reconnect on physical iOS and
   Android devices before committing to the broader roadmap.
