# Rift mobile

Companion app for [Rift](../README.md) (Phases 1–3 of [the mobile plan](../docs/mobile-remote-plan.md)):
pairs with the Rift remote gateway on your computers, shows their windows, tabs, panes and live
command blocks, runs commands (at once, or after a confirmation when asked to), sends control
keys, shows and types into full-screen programs (vim, less, htop), pages through earlier
output, answers what a running command or coding agent asks (a permission menu, a password),
and notifies when a command finishes or fails or starts asking. On the local network it connects directly; elsewhere it goes through
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

Pair from Now (first run) or Computers (**+**): scan the QR code that Rift shows (Settings › Remote access › Pair a
phone, or `rift-terminal remote pair`), paste the `rift-remote://pair?…` link, or open the link
on the device. The app asks before pairing.

## Screens

The screens follow the Claude Design boards of the mobile redesign: the phone-first proposal
(P1 Now, P2 Reply, P3 Pane, P4 Computers, P5 Run, P6 Lock) and, where it replaces none, the
earlier ones (Empty, Pair, PairConfirm, PaneNoWrap, FullScreen, BlockSheet, Confirm, Settings,
ControlKeys, the chip sheets);
colours are the tokens in `src/ui/palette.ts`, icons are system symbols (`src/ui/Icon.tsx`).
The boards' faces are embedded at build time (`expo-font` `fonts` in `app.json`,
`assets/fonts/`, OFL): Inter as static Regular / Medium / SemiBold instances of the repo's
`assets/fonts/Inter-Variable.ttf` (fontTools `varLib.instancer`, wght 400/500/600, opsz 14;
Android cannot pick weights from a variable font) and JetBrains Mono Regular / Medium / Bold.
Each weight is its own family (`ui(weight)`, `mono`, `monoMedium` in `src/ui/theme.ts`) and no
style sets `fontWeight`, which Android ignores on a custom family (`test/fonts.test.ts`).
New fonts need a native rebuild (`npx expo run:ios` / `run:android`).

Tabs: **Now**, **Computers**, **Settings** (`src/now.ts` holds their models).

- **Now** (home): what needs you, what runs, what finished, on every connected computer.
  *Needs you · N* (feature `waiting`): a coding agent's menu as a card with its answers as
  buttons ("Yes", "Yes, always", "No…"; "No…" and the card open the answer page); a password or
  other question as a row with **Answer**. *Running · N* (feature `running` for the start time
  and the last line): command, elapsed, its latest line, "Notifies when done". *Finished*: this
  run's finished commands, failures first, then newest; a row opens the pane at that block.
  **❯ Run** opens the Run sheet. With none paired it is the first-run page (no tab bar there).
- **Answer** (the P2 Reply board): "claude is asking", where, **Open pane**; the question, the
  agent's lines above it (a diff drawn as one), then the answers: a numbered entry is sent as
  its digit, an unnumbered one as arrow keys from the selected entry and Enter
  (`src/answer.ts`); the last entry when it is "No" is a field: empty it sends the No, with
  text it sends the No, waits 0.6 s, then the text and Enter ("No, and tell claude what to do").
  A password is a secure field sent as `text` and Enter, never shown, logged or kept. An answer
  goes only to the question it was shown for (the pane must still ask it). View-only devices
  get no answers. Ask before running asks for Face ID first; otherwise nothing confirms.
- **Computers** (the P4 board): a card per paired computer; the connected one shows its tabs
  (name, last command, running / failed / needs you), the others say "N tabs · LAN/Relay"
  (tap to show their tabs), offline ones are dimmed with "seen 2 h ago". **+** pairs. Settings ›
  Paired computers › a computer opens its page (its tabs and its notification bell).
- **Reopening**: a cold start goes back to the pane that was open last (`rift.last` in the
  keychain); otherwise Now.
- **Pane** (the P3 board): back, the computer, wrap and search; the window's tabs as a strip
  (the pane count on the current tab, a red ● on a tab whose last command failed); "Pane 1 of 2 ·
  swipe for claude" with dots, and a sideways swipe (or the screen reader's next / previous
  action) moves between the tab's panes. A pane that asks shows "claude is asking: … Answer".
  Blocks only: the successes before the newest failure (or the running command) fold into
  "✓ N earlier commands succeeded · Show"; a failed command is a card with Run again, Copy error
  and All N lines; the running one a card with Notify when done and Stop (^C). While a
  full-screen program runs (the alternate screen: vim, htop, less) the pane shows its screen
  ("htop is full screen · showing its screen"), the keys wrapped under it and "Show keyboard";
  when it quits the blocks come back. **Wrap long lines** for this pane (Settings › Output ›
  Wrap long lines for all; off, each block's output scrolls sideways with a position bar) and
  **Search output** (filters the blocks by command or output). Above the command line:
  suggestions (the desktop's completions for what is typed and this pane's history, feature
  `complete`; else its recent commands), then the folder button (folder and branch with
  ↑ahead ↓behind and a dot for uncommitted changes; it opens the folder, branch and changed-files
  menus), the field (Return runs) and **⌃**, which shows the key row in place of the
  suggestions. The keyboard's smart dashes and quotes are undone in the command line.
- **Run** (the P5 sheet, from Now's **❯ Run**): the pane it runs in ("studio-mac › rift › pane 1
  ▾", the last pane open by default), the field, *Complete* (the desktop editor's completions,
  each the whole line, with its kind) and *Ran here before* (the pane's commands, newest first,
  with how long ago, then the shell history the desktop imported). Return runs it there (the
  Confirm sheet first with Ask before running) and opens the pane.
- **Chip menus** (a computer whose welcome lists `chips`; otherwise the folder button does
  nothing), as the desktop's: the folder opens the parent and sub-folders (searchable) and picking one
  runs `cd ..` / `cd ./<name>`; the branch chip the local branches, the current one checked,
  and picking one runs `git checkout <name>` (a new name typed in the search: `git checkout -b`);
  the changed-files chip the code review panel's files, staged first, and a file opens its diff,
  read-only. Names from the computer are untrusted: they are shell-quoted as the desktop quotes
  them (`src/chips.ts` `shellQuote`), never run unquoted. The commands go through Run (the
  Confirm sheet first with Ask before running), without a "Notify me".
- **Show keys above the command line** (Settings › Output, off by default): whether the pane
  opens with the key row shown (**⌃** switches it; ^C stays on the running card: Stop); under a
  full-screen program the keys always show.
- **Running**: Run sends at once; Settings › Running commands › Ask before running (off by
  default) shows the Confirm sheet first. With Notify me when my commands finish (on by
  default) every command run from this phone asks to be told when it finishes; the bell on a
  running block switches that for one command (and on a computer with `notify_off` also
  drops the computer's own wait).

## Connecting

- Order: the path that last worked, else each LAN address (1.5 s to connect and finish the
  handshake while a relay is there to fall back on, 8 s without), then the relay
  (`<r>/connect?id=<routing id>`). Host status and the pane banner name the path
  ("LAN 192.168.1.29", "Relay relay.example.com"). A LAN address that answers with
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
  screenshots while the lock is on), and, with "Ask before running" and "Run with Face ID" on,
  before Run and before the keys that submit a line (`enter` with any modifiers, `ctrl+m`,
  `ctrl+j`). "Run with Face ID" shows only with the lock and Ask before running on. Settings
  saved before the redesign had "Confirm every command" (on by default, always stored): it is
  not carried over, so Ask before running starts off. On a full-screen program's screen,
  while the pane runs a program, one authentication holds until the view closes
  or the app leaves the foreground, so Enter in vim does not ask every time; at the pane's
  prompt a submit key runs a command and asks every time. Input typed while it asks waits
  behind it. A device without a passcode or secure lock screen is let in. Rules: `src/lock.ts`.

## Terminal, keys and output (Phase 3)

What a computer serves beyond Phase 2 comes in `welcome.features` (`screen`, `text`, `output`);
against an older Rift the app hides what needs them.

- **Full-screen programs** (pane screen, `screen`): the app asks for the screen in mode `auto`,
  so a full-screen program switches the pane to its screen by itself and back to blocks when
  it exits; there is no switch. A reconnect or re-subscribe keeps the screen on display until
  the next one comes (or for 5 s, after which the program is taken to have ended). The grid is
  monospace rows with the program's colours (null: terminal text on the pane's ground), bold, italic, underline, dim,
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
  (`busy`): the view says so. In the command line itself, a hardware
  keyboard's ctrl/alt combinations that are keys, Escape and F-keys go to the pane as keys, and
  Tab / Shift+Tab too while a program runs or the line holds text (otherwise Tab moves the
  focus). Mapping: `src/keyboard.ts`.
- **Keys** (Settings › Output › Keys, or "⌨" at the end of the row): a preview, the row (drag
  the ≡ handle to reorder; screen readers get Move up / Move down), and Add chips from a
  catalogue (`src/controlKeys.ts`); the default row is ^C esc tab ctrl ↑ ↓. ctrl and alt are sticky for the
  next key, whatever it is (`ctrl+enter`, `alt+left`, `ctrl+[` is esc, `ctrl+@` is ctrl+space);
  on a key the protocol has no name for the modifier is dropped and the plain key sent.
  Two catalogue keys are actions, not keys: **+tab** opens a new tab in the pane's window and
  **+pane** splits the pane (feature `open`; disabled on an older Rift), both starting in the
  pane's folder; the app then shows the new pane in place of this one.
- **Block actions** ("···" on a finished block, or a long press): Run again (through the
  Confirm sheet when Ask before running is on), Edit, then run (into the command line, asking
  first when that holds other text), Copy command, Copy output, Share output; Stop (ctrl+c)
  for a running one. A running block shows the bell instead.
- **Earlier output** (`output`): a block whose tail starts later offers "Load earlier output",
  500 lines a press and at most 5,000 per block (`src/earlier.ts`); lines the computer's
  scrollback no longer holds are said to be gone. An answer for another pane or block, or
  starting past the range asked for, fails the load.

## Notifications

Settings › Other commands on your computers › Notify for: Nothing, Failures (an exit status
other than 0) or Everything, for commands that ran longer than "Only if it ran longer than"
(10 s, 30 s, 1 min or 5 min; 30 s by default), when you are not looking at that pane. Each
paired computer's page has its own bell (its notifications). A "Notify me" (every command run
from this phone, by default, and a running block's bell) always notifies. The app's own
notifications and the push registration follow the same rules (`src/notify.ts`). Settings saved
before Phase 3 move over: "Command finished or failed" on becomes Finished or failed commands.

## Needs you alerts and the lock screen (the P6 board)

- **Alerts**: while the app runs (on screen, or the short while iOS keeps it alive after), a
  question a pane starts asking posts "claude is asking on studio-mac" with the question; not
  for the pane on screen, not for a computer whose notifications are off, not for the questions
  already waiting when it connected (Now shows those). A menu with a Yes and a No gets the
  buttons **Yes**, **No** and **Open** (category `rift-ask`), anything else **Open** only
  (`rift-ask-open`). Yes and No need the phone unlocked (Face ID), open the app and send the
  menu's first "Yes…" or "No…" once the pane still asks that question (and, with Ask before
  running on, Face ID in the app first); otherwise the question opens.
- **Live Activity** (iOS 16.2+): each "Notify me" wait (a command run from this phone with Notify
  me when my commands finish, or Notify when done on the running card) shows the command, where
  it runs, a clock the system ticks by itself (`Text(timerInterval:)`) and its latest line on the
  lock screen and in the Dynamic Island; its `done` turns it into ✓ or ✕ N, which stays 15
  minutes. The widget extension `RiftLive` (`targets/RiftLive`, added by
  `plugins/withLiveActivity.js`) needs no entitlement, so a free Apple ID signs it like the app;
  the app drives it through `modules/rift-live` (`src/live.ts` plans it).
- **The APNs limit**: without Apple's developer program (`RIFT_NO_APNS=1`) there is no APNs, so
  nothing reaches the phone while the app is suspended except through ntfy: no Needs you alert
  (the computers' push and ntfy alerts are about finished commands only), and a Live Activity
  keeps ticking but shows its last line and is not ended by a `done` that comes while the app
  is suspended (iOS ends it after 8 hours; opening the app ends it at once). With an APNs key the
  same would need ActivityKit push tokens sent to the computer and a Needs you push from it.

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
only with "Show the command", the command line; never output. Settings › While the app is
closed says which way notifications come. Without a token (no push project in the
build, or notifications not allowed) the app notifies only while it runs, unless ntfy is on.

### ntfy (free, no push setup)

[ntfy](https://ntfy.sh) delivers the same alerts through its own free app, so it works in a
build without Apple's developer program or a push project (`push.ntfy`, protocol § Push; needs
a computer whose welcome lists `push_ntfy`):

1. Install the ntfy app (App Store, or Google Play / F-Droid).
2. In Rift: Settings › While the app is closed › turn on **Alerts through ntfy**. The app makes a random
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
push; Settings and that computer's page say "Update Rift on …". **New** makes a new topic
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
only with "Show the command", the command line; never output. The topic name is the
secret: anyone who knows it can read the alerts. The computers refuse `localhost` and loopback,
link-local and multicast addresses as servers; LAN and Tailscale (100.64/10) addresses work.

**iPhone with a free Apple ID.** A personal team cannot sign the Push Notifications capability
that expo-notifications adds (`aps-environment` in `ios/Rift/Rift.entitlements`). Build with
`RIFT_NO_APNS=1`, which makes `plugins/withoutApns.js` leave it out:

```sh
RIFT_NO_APNS=1 npx expo prebuild -p ios --clean
npx expo run:ios --device --configuration Release
```

Expo push is then unavailable; ntfy works (Settings › While the app is closed). `ios/` is generated
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
                 # key mapping, terminal layout, earlier output, control keys, prompt chips,
                 # Now / pane / answer / Live Activity models (phone.test.ts), embedded fonts
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
  iOS (Swift), Android (Kotlin) and the web. `modules/rift-live/` — the Live Activity (iOS,
  Swift ActivityKit); `targets/RiftLive/` — its widget extension, added by
  `plugins/withLiveActivity.js` (which checks the two copies of `RiftLiveAttributes.swift` match).
- `src/fleet.ts` — every paired desktop with its live connection, pairing, run/keys with
  acks, activity and notification decisions (`notify.ts`); `src/connectivity.ts` and
  `src/lock.ts` — network/background and app-lock state machines; `src/stores.ts` — device key,
  hosts and settings over a key-value store; `src/runtime.ts` wires them to expo-secure-store,
  expo-crypto, expo-notifications, expo-network, expo-local-authentication,
  expo-screen-capture and AppState.
- `src/chips.ts` — the prompt chips and their menus; `src/now.ts` — Now's lists, the pane's
  folds and pager, Computers' tab states; `src/answer.ts` — what an answer sends;
  `src/live.ts` — the Live Activity plan.
- `src/app/` — expo-router screens: tabs Now / Computers / Settings (iOS: the system tab bar
  via native tabs, `(tabs)/_layout.ios.tsx`, which iOS 26 draws as the floating glass bar;
  Android and web: the same floating bar drawn by `src/ui/FloatingTabBar.tsx`), Paste a pairing
  link (`add`), Pair a computer (`scan`), pairing confirmation (`pair`, also the target of
  `rift-remote://pair` links), Host (a computer's tabs and bell), Pane with its command line and
  full-screen view, Answer (`answer`), Run sheet (`run`), Confirm sheet, block actions sheet
  (`block`), chip menus (`chips`, `diff`), Keys (`control-keys`); `src/ui/LockGate.tsx` — the
  lock screen and privacy cover.
- `src/ui/palette.ts` — the DESIGN.md tokens and the text/background pairs the app uses, each
  held to 4.5:1 by `test/contrast.test.ts`.
- The device key, paired desktops and settings live in the iOS keychain / Android keystore
  (expo-secure-store); on web they are kept in memory only.
