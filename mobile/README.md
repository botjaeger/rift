# Rift mobile

Companion app for [Rift](../README.md) (Phases 1–2 of [the mobile plan](../docs/mobile-remote-plan.md)):
pairs with the Rift remote gateway on your computers, shows their windows, tabs, panes and live
command blocks, runs commands after a confirmation, sends control keys, and notifies when a
command finishes. On the local network it connects directly; elsewhere it goes through the
gateway's relay, which only forwards encrypted traffic.

The wire format is [docs/remote-protocol.md](../docs/remote-protocol.md) (version 2): a binary
WebSocket carrying `Noise_XX_25519_ChaChaPoly_BLAKE2s` with the gateway key pinned at pairing,
the same over the relay. The decoder tests run over the shared fixtures in
`../crates/rift-remote/fixtures/`.

## Run

```sh
pnpm install
pnpm start          # Expo dev server; development build (expo run:ios / run:android) or web
```

Pair from the Hosts tab: scan the QR code that Rift shows (Settings › Remote access › Pair a
phone, or `rift-terminal remote pair`), paste the `rift-remote://pair?…` link, or open the link
on the device. The app asks before pairing.

## Connecting

- Order: the path that last worked, else each LAN address (1.5 s to connect and finish the
  handshake while a relay is there to fall back on, 8 s without), then the relay
  (`<r>/connect?id=<routing id>`). Host status and the pane banner name the path
  ("LAN · 192.168.1.29", "Relay · relay.example.com"). A LAN address that answers with
  another key than the paired one is skipped; the host stops with "key mismatch" only when the
  relay, or every path, does. A network change tries it again.
- Network changes (expo-network): every socket is dropped and reconnected at once, LAN first, so
  coming home from cellular goes back to the direct path. Without a network hosts show Offline
  and nothing is retried until it returns.
- Background: sockets close 30 s after the app leaves the foreground (3 minutes while a
  "Notify me" waits) and reconnect when it returns. Rules: `src/connectivity.ts`. That is the
  most the app keeps: the system may end it sooner. Android 15 and later block a background
  app's network within seconds of it leaving the screen (`dumpsys netpolicy`:
  `blocked=APP_BACKGROUND`), and iOS suspends apps soon after; either way the app reconnects
  when it returns. Finish alerts while it is closed come through push (below). A "Notify
  me" waits at most 24 h, and is dropped when its pane is gone or the computer's Rift
  restarted (`done` is not replayed).
- App lock (Settings › Security, expo-local-authentication): Face ID / fingerprint or the device
  passcode on launch and after 5 minutes away, a cover in the app switcher (on Android also
  FLAG_SECURE through expo-screen-capture, which blanks the Recents thumbnail and blocks
  screenshots while the lock is on), and, with "Confirm every command" on, before Run and
  before the keys that submit a line (`ctrl+m`, `ctrl+j`, `enter`). A device without a
  passcode or secure lock screen is let in. Rules: `src/lock.ts`.

## Push (alerts while the app is closed)

With notifications allowed, the app asks for an Expo push token and sends it to every paired
computer (`push`, [protocol § Push](../docs/remote-protocol.md#push-alerts-while-the-app-is-closed));
"Notify me" also tells the computer (`notify`). The computer then alerts through the Expo push
service by itself, also while the app runs (the app posts nothing of its own for it, and drops an
alert about the pane on screen). An alert carries the computer's name, the exit status and,
only with "Show command in notification", the command line; never output. Settings ›
Notifications says which way notifications come. Without a token (no push project in the
build, or notifications not allowed) the app notifies only while it runs.

A build needs, once (free, except Apple's developer program for iOS):

1. An Expo account: `npx expo login`, then `npx eas-cli init` here (writes
   `extra.eas.projectId` to `app.json`).
2. Android: a Firebase project with an Android app `dev.botjaeger.rift`; put its
   `google-services.json` here, set `"googleServicesFile": "./google-services.json"` under
   `android` in `app.json`, and upload the project's FCM V1 service-account key with
   `npx eas-cli credentials` (Android › Push notifications).
3. iOS: an Apple Developer Program membership; `npx eas-cli credentials` creates the APNs key.
4. Rebuild the app (`npx expo run:android` / `run:ios`).

## Checks

```sh
pnpm typecheck   # tsc --noEmit (strict)
pnpm test        # vitest: Noise vectors + snow interop, framing, client, relay, adverse network,
                 # connectivity, lock, fleet, stores, pairing, notifications, text contrast
pnpm lint        # eslint
npx expo install --check && npx -y expo-doctor
npx expo export -p web   # the web bundle must build
```

On a device, open `rift://diagnostics` for the transport benchmark (Noise
handshake, 1 MiB sealed and opened in 65 000-byte chunks on this phone's
JavaScript engine).

`pnpm-workspace.yaml` lets the Expo SDK 57 patch releases through pnpm's
one-day minimum release age by exact version only.

## Layout

- `src/noise/` — Noise XX (`noise.ts`, rev 34, on @noble X25519 / ChaChaPoly / BLAKE2s) and byte
  helpers (`bytes.ts`). Vectors: `test/vectors/` (cacophony) and the Rust side's
  `fixtures/noise/xx.json` (snow).
- `src/protocol/` — wire types (`types.ts`), chunk framing (`framing.ts`), paths and the relay
  routing id (`relay.ts`), one gateway connection with handshake, LAN/relay failover,
  reconnect, resume and gap detection (`client.ts`), the bounded pane reducer (`store.ts`),
  100 ms batched store (`batch.ts`).
- `src/fleet.ts` — every paired desktop with its live connection, pairing, run/keys with
  acks, activity and notification decisions (`notify.ts`); `src/connectivity.ts` and
  `src/lock.ts` — network/background and app-lock state machines; `src/stores.ts` — device key,
  hosts and settings over a key-value store; `src/runtime.ts` wires them to expo-secure-store,
  expo-crypto, expo-notifications, expo-network, expo-local-authentication,
  expo-screen-capture and AppState.
- `src/app/` — expo-router screens: tabs Hosts / Activity / Settings (iOS system tab bar via
  native tabs, `(tabs)/_layout.ios.tsx`), Pair a desktop (`add`), scanner (`scan`), pairing
  confirmation (`pair`, also the target of `rift-remote://pair` links), Host, Pane with
  composer, Confirm sheet; `src/ui/LockGate.tsx` — the lock screen and privacy cover.
- `src/ui/palette.ts` — the DESIGN.md tokens and the text/background pairs the app uses, each
  held to 4.5:1 by `test/contrast.test.ts`.
- The device key, paired desktops and settings live in the iOS keychain / Android keystore
  (expo-secure-store); on web they are kept in memory only.
