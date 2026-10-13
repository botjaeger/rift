# Rift mobile

Companion app for [Rift](../README.md) (Phases 1–3 of [the mobile plan](../docs/mobile-remote-plan.md)):
pairs with the Rift remote gateway on your computers, shows their windows, tabs, panes and live
command blocks, runs commands after a confirmation, sends control keys, shows and types into
full-screen programs (vim, less, htop), pages through earlier output, and notifies when a
command finishes or fails. On the local network it connects directly; elsewhere it goes through
the gateway's relay, which only forwards encrypted traffic.

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
  before the keys that submit a line (`enter` with any modifiers, `ctrl+m`, `ctrl+j`). In the
  Terminal view, while the pane runs a program, one authentication holds until the view closes
  or the app leaves the foreground, so Enter in vim does not ask every time; at the pane's
  prompt a submit key runs a command and asks every time. Input typed while it asks waits
  behind it. A device without a passcode or secure lock screen is let in. Rules: `src/lock.ts`.

## Terminal, keys and output (Phase 3)

What a computer serves beyond Phase 2 comes in `welcome.features` (`screen`, `text`, `output`);
against an older Rift the app hides what needs them.

- **Blocks | Terminal** (pane screen, `screen`): the app asks for the screen in mode `auto`, so a
  full-screen program switches the pane to Terminal by itself and back to Blocks when it exits.
  Picking Terminal asks for the screen always (`on`) and stays. A reconnect or re-subscribe
  keeps the screen on display, marked "not up to date", until the next one comes (or for 5 s,
  after which the program is taken to have ended); a picked view stays. The grid is monospace rows with
  the program's colours (null: the app's text on its background), bold, italic, underline, dim,
  strikethrough and an inverse cursor; the font fits the width between 7 and 14 pt, a wider
  screen scrolls sideways. Each row is text for screen readers; a diff re-renders only its rows.
  Layout: `src/terminal.ts`, view: `src/ui/TerminalView.tsx`.
- **Typing** (operate, `text`): the local Expo module `modules/rift-keys` (`KeyCaptureView`:
  Swift `UIKeyInput` + `pressesBegan`, Kotlin input connection + `dispatchKeyEvent`, DOM events on
  the web) takes the soft and hardware keyboard. Text goes out as `text`, merged per frame and at
  most every 50 ms; Return, Backspace, arrows, Escape, F-keys and ctrl/alt combinations as
  `keys`; Cmd / Meta stays with the system. Characters composed with AltGr or Option (`@`, `{`,
  `|`, `™`) are text, not alt+key. Held named keys repeat (iOS: after 0.5 s, every 0.05 s; Android
  and the web repeat by themselves). Input goes out in order without waiting for answers; keys
  and text together keep to the gateway's 20 messages a second (`src/protocol/client.ts`
  paces them, queued, never dropped), and what waits merges with what is typed meanwhile. The
  keyboard button shows and hides the soft keyboard. Text to a pane at its prompt is refused
  (`busy`): the view points to the command line. In the command line itself, a hardware
  keyboard's ctrl/alt combinations that are keys, Escape and F-keys go to the pane as keys, and
  Tab / Shift+Tab too while a program runs or the line holds text (otherwise Tab moves the
  focus). Mapping: `src/keyboard.ts`.
- **Control keys** (Settings › Control keys): pick and order the row above the command line and
  under the terminal from a catalogue (`src/controlKeys.ts`); ctrl and alt are sticky for the
  next key, whatever it is (`ctrl+enter`, `alt+left`, `ctrl+[` is esc, `ctrl+@` is ctrl+space);
  on a key the protocol has no name for the modifier is dropped and the plain key sent.
- **Block actions** ("⋯" on each block): copy command or output, share output, edit the command
  into the command line (asking first when that holds other text), run it again (through the
  Confirm sheet), stop it (ctrl+c), notify me when it finishes.
- **Earlier output** (`output`): a block whose tail starts later offers "Load earlier output",
  500 lines a press and at most 5,000 per block (`src/earlier.ts`); lines the computer's
  scrollback no longer holds are said to be gone. An answer for another pane or block, or
  starting past the range asked for, fails the load.

## Notifications

Settings › Notifications: Off, Failed commands (an exit status other than 0) or Finished or
failed commands, for commands that ran at least the "Notify after" time (10 s, 30 s, 1 min or
5 min; 30 s by default), when you are not looking at that pane. Each paired computer's page has
its own Notifications switch. "Notify me when it finishes" always notifies. The app's own
notifications and the push registration follow the same rules (`src/notify.ts`). Settings saved
before Phase 3 move over: "Command finished or failed" on becomes Finished or failed commands.

## Push (alerts while the app is closed)

With notifications allowed, the app asks for an Expo push token and sends it to every paired
computer (`push`, [protocol § Push](../docs/remote-protocol.md#push-alerts-while-the-app-is-closed));
"Notify me" also tells the computer (`notify`); one asked while disconnected is sent after the
next welcome. The registration carries the policy: no `long_ms` alerts while notifications are
off or the computer's switch is off, `failures_only` for Failed commands. The computer then
alerts through the Expo push service by itself, also while the app runs; for a finished command
its registration covers (a "Notify me" it received, or a long one by `long_ms` and
`failures_only`) the app posts nothing of its own, for any other it notifies itself, and it drops an
alert about the pane on screen. An alert carries the computer's name, the exit status and,
only with "Show command in notification", the command line; never output. Settings ›
Notifications says which way notifications come. Without a token (no push project in the
build, or notifications not allowed) the app notifies only while it runs, unless ntfy is on.

### ntfy (free, no push setup)

[ntfy](https://ntfy.sh) delivers the same alerts through its own free app, so it works in a
build without Apple's developer program or a push project (`push.ntfy`, protocol § Push; needs
a computer whose welcome lists `push_ntfy`):

1. Install the ntfy app (App Store, or Google Play / F-Droid).
2. In Rift: Settings › Notifications › turn on **Alerts through ntfy**. The app makes a random
   topic (`rift-` and 22 random characters) on `https://ntfy.sh` and sends it to every computer.
   To self-host, enter your server's address (https only) in the field under the switch; the
   topic stays, but subscribe again in ntfy on the new server.
3. Tap **Subscribe in ntfy**. On Android it opens ntfy on the topic (`ntfy://<host>/<topic>`,
   [ntfy docs › Integrations](https://docs.ntfy.sh/subscribe/phone/)); without ntfy installed it
   says to install it from Google Play or F-Droid first. ntfy for iPhone has no subscribe link,
   so there the button copies the topic: in ntfy tap **+**, paste it as the topic (for a
   self-hosted server also turn on "Use another server" and enter it), then **Subscribe**.
   **Copy** copies the topic on either.

With ntfy on, ntfy replaces Expo push for every computer that supports it (the app sends those
no token), so each command alerts once. A computer whose Rift is too old for ntfy keeps Expo
push; Settings and that computer's page say "Update Rift on …". **New topic** makes a new one
and sends it to every computer: subscribe to it in ntfy again; computers that get it stop posting
to the old topic; one that is offline may post there until it reconnects, and one unpaired while
offline may keep posting there.
Turning the switch off keeps the topic, so turning it on again needs no new subscription.

Tapping an alert in ntfy opens the pane in Rift (`Click: rift://host/<key>/pane/<id>`). The
computer alerts through ntfy also while Rift runs, so the app posts no notification of its own
for what that covers. While Rift is on screen (not in the background; a Face ID prompt still
counts as on screen) the computer sends no ntfy alert about the pane the app follows unless you
asked with "Notify me", and the app notifies about that one itself when you are not looking at
it. In the background (the app tells the computer, `visible`), or once the connection has been
silent for 20 s, ntfy alerts for it too.

ntfy.sh (or your server) sees, per alert, the computer's name and IP address, the exit status
("Exit N" or "Finished"), a link to the pane (the computer's public key and pane number) and,
only with "Show command in notification", the command line; never output. The topic name is the
secret: anyone who knows it can read the alerts. The computers refuse `localhost` and loopback,
link-local and multicast addresses as servers; LAN and Tailscale (100.64/10) addresses work.

**iPhone with a free Apple ID.** A personal team cannot sign the Push Notifications capability
that expo-notifications adds (`aps-environment` in `ios/Rift/Rift.entitlements`). Build with
`RIFT_NO_APNS=1`, which makes `plugins/withoutApns.js` leave it out:

```sh
RIFT_NO_APNS=1 npx expo prebuild -p ios --clean
npx expo run:ios --device --configuration Release
```

Expo push is then unavailable; ntfy works (Settings › Notifications then shows ntfy delivery). `ios/` is generated
(git-ignored), so run the prebuild again with the variable after anything that regenerates it.
`RIFT_NO_APNS=1 npx expo config --type introspect` shows the entitlements without writing files.

A self-hosted server delivers to iPhones promptly only with `upstream-base-url` set
([ntfy docs › iOS instant notifications](https://docs.ntfy.sh/config/#ios-instant-notifications)).

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
                 # connectivity, lock, fleet, stores, pairing, notifications, text contrast,
                 # key mapping, terminal layout, earlier output, control keys
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
  reconnect, resume, gap detection, screen mode and output requests (`client.ts`), the bounded
  pane reducer and the screen reducer (`store.ts`), 100 ms batched store (`batch.ts`).
- `src/keyboard.ts`, `src/terminal.ts`, `src/earlier.ts`, `src/controlKeys.ts` — key events to
  `keys` / `text`, the terminal grid's layout, earlier-output paging, the control-key catalogue.
- `modules/rift-keys/` — local Expo module (autolinked from `modules/`): `KeyCaptureView` for
  iOS (Swift), Android (Kotlin) and the web.
- `src/fleet.ts` — every paired desktop with its live connection, pairing, run/keys with
  acks, activity and notification decisions (`notify.ts`); `src/connectivity.ts` and
  `src/lock.ts` — network/background and app-lock state machines; `src/stores.ts` — device key,
  hosts and settings over a key-value store; `src/runtime.ts` wires them to expo-secure-store,
  expo-crypto, expo-notifications, expo-network, expo-local-authentication,
  expo-screen-capture and AppState.
- `src/app/` — expo-router screens: tabs Hosts / Activity / Settings (iOS system tab bar via
  native tabs, `(tabs)/_layout.ios.tsx`), Pair a desktop (`add`), scanner (`scan`), pairing
  confirmation (`pair`, also the target of `rift-remote://pair` links), Host, Pane with
  composer and Terminal view, Confirm sheet, block actions sheet (`block`), Control keys;
  `src/ui/LockGate.tsx` — the lock screen and privacy cover.
- `src/ui/palette.ts` — the DESIGN.md tokens and the text/background pairs the app uses, each
  held to 4.5:1 by `test/contrast.test.ts`.
- The device key, paired desktops and settings live in the iOS keychain / Android keystore
  (expo-secure-store); on web they are kept in memory only.
