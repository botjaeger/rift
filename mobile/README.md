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

Pair from Computers (the first-run page, or **+**): scan the QR code that Rift shows (Settings › Remote access › Pair a
phone, or `rift-terminal remote pair`), paste the `rift-remote://pair?…` link, or open the link
on the device. The app asks before pairing.

Each launch opens on the splash (`src/ui/Splash.tsx`, timeline in `src/splash.ts`): the native
splash (`expo-splash-screen`) is the bare `#181818` ground, then the mark `>▌` blinks once and
"rift" is typed after it, and the splash fades to the app in under a second. With Reduce Motion
the finished prompt shows at once.

## Over-the-air updates

Release builds carry `expo-updates` on the `production` channel (`app.json` › `updates`). The
app does its own checks (`src/ota.ts`; the native one runs only to recover from a launch that
crashed, `checkAutomatically: ON_ERROR_RECOVERY`): at launch once the settings are read, every
15 minutes and on each return to the foreground, while Settings › About › **Check
automatically** is on (the default); **Check now** checks on demand. A check that has not
answered in 10 s is reported as failed. Once an update has downloaded, the app asks once to **Restart**
(after any unlock); **Later** applies it at the next start, and About offers **Restart now**
while one is waiting. Development builds and the web never update.

About names the version and the update it runs ("0.2.0 · update 1", from `src/release.ts`,
which `/release` writes). The release notes are `CHANGELOG.md`, embedded as
`src/changelog.data.ts` (`pnpm changelog`; `pnpm ota` regenerates it, and a test fails while
the two differ): after a new update or build the app shows **What's New** once (the notes since
the release it last showed; a first install shows none), and About › **What's New** lists them
all.

```sh
eas login           # once
pnpm ota --message "what changed"   # changelog, then eas update --channel production
```

An update reaches only builds with the same native fingerprint (`runtimeVersion` policy
`fingerprint`): a change to native code, a config plugin or a native dependency needs a new build,
not an update.

## Screens

The screens follow the Claude Design boards of the mobile redesign: the phone-first proposal
(P2 Reply, P3 Pane, P4 Computers, P6 Lock) and, where it replaces none, the
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

**Computers** is the home page; there is no tab bar. Its gear opens **Settings**, a pushed page;
its **+** pairs a computer. `src/now.ts` and `src/paneView.ts` hold the models.

- **Answer** (the P2 Reply board): "claude is asking", where, **Open pane**; the question, the
  agent's lines above it (a diff drawn as one), then the answers: a numbered entry is sent as
  its digit, an unnumbered one as arrow keys from the selected entry and Enter
  (`src/answer.ts`); the last entry when it is "No" is a field: empty it sends the No, with
  text it sends the No, waits 0.6 s, then the text and Enter ("No, and tell claude what to do").
  A password is a secure field sent as `text` and Enter, never shown, logged or kept. An answer
  goes only to the question it was shown for (the pane must still ask it). View-only devices
  get no answers. Ask before running asks for Face ID first; otherwise nothing confirms. The page
opens from the **No…** answer on Computers, from the line under a pane's header for a question its
card cannot answer (a password, other input, a view-only device), and from an alert.
- **Computers** (home, the P4 board): the title with a gear (**Settings**) and **+** (**Pair a
  computer**). No search bar. Chips under the title filter the tabs of every computer by state:
  "N needs you", "N working", "N idle" (a tab whose last command failed counts as idle). A chip shows
  only while a tab is in its state; a second tap clears it; "needs you" stands out while nothing
  filters. Below, a card per paired computer (with a filter on, only those with a matching tab):
  a dot (green while connected), the name, "view only" when this phone cannot run commands, and a
  line: "LAN · 4 tabs" or "Relay · 1 tab", "Connecting…", "Offline · seen 2h ago" (or "not
  connected yet"). On the card's right: **New tab** (the icon; a connected computer that this phone
  may operate and that offers `open`, or `start` while Rift is closed), **Retry** while it is not
  connected, **Pair again** when it refused this phone's key (other failures say why under the
  name). A connected card lists its tabs: a tile (✳ in orange for a coding agent, `>_` for a
  shell), the title, the state at the right ("Needs you · 2m", "Working 3:56", "2 failed",
  "Idle 12m"; the idle time only for an agent the app saw working) and a line (program, folder,
  branch, or the last command; with several computers paired a question shows its first line). A
  tap opens the tab's pane. With one computer paired, a needs-you tab whose agent asks a menu of up to
  three distinct entries answers in place: the question and pills (**Yes**, **Always**, **No**; the
  first entry is filled), and **Open**. A pill sends that entry (Face ID first when Settings asks);
  **No** opens the Answer page with its field. With none paired it is the first-run page (**Scan
  the code**, **Paste a pairing link**). Settings › Computers › a computer opens its page.
- **Tabs** (a sheet, from the pane's header title): every tab of that computer, needs you first,
  then failed, working, idle. Chips filter: All, Needs you, Working, Failed, each with its count.
  A row: tile, title (**Here** on the open tab), the last line, a state pill ("Needs you", "✕ 2
  failed", "Working 3:56", "Idle 12m") and the pane count. A tap goes to that tab's pane in place of
  this one (Back still leads where it led). **New tab** at the foot.
- **Panes** (a sheet, from the pane picker): the tab's panes, each with its mark (✳ or `>_`),
  name, last line (the question, the latest output line, the folder or the last command) and state
  (Running, Needs you, ✕ exit, Idle); the open one is tinted. A tap opens that pane in place of this
  one.
- **Open a tab while Rift is closed** (gateways with `start`, which the computer offers only when
  its user turned on *Phones can open tabs while Rift is closed*): with no Rift window open,
  **New tab** (a computer's card, or the Tabs sheet) asks Face ID (or the passcode) first, then
  opens a shell the computer keeps and lists under "Rift is closed · Tabs" until Rift opens and
  shows it as a tab. View-only devices do not get the button.
- **Reopening**: a cold start goes back to the pane that was open last (`rift.last` in the
  keychain); otherwise Computers.
- **Pane** (the P3 board): commands run from its command line; there is no separate Run page.
  The header: Back (its badge counts the computer's other panes that need you); the tab's name
  with "studio-mac · tab 2 of 5" and a chevron, which opens the Tabs sheet; **Search output**
  (filters the blocks by command or output, "N of M"; not under a full-screen view); and **⋯**.
  Under it the pane picker: the pane's mark, name and "· 1/2", a dot and count for the tab's
  other panes that need you, which opens the Panes sheet; and the **Terminal view** button
  (gateways with `screen`). A sideways swipe across the blocks (or the picker's Next pane /
  Previous pane screen-reader actions) also moves to the tab's next or previous pane. **⋯** holds
  two switches: **Wrap long lines** for this pane (Settings › Panes › Wrap long lines sets the
  default; off, each block's output scrolls sideways with a position bar) and **Notifications**
  for this computer.
  Blocks are cards (below); the successes before the newest failure (or the running command) fold
  into "✓ N earlier commands succeeded · Show", and a pane with older blocks than loaded offers
  **Earlier blocks**. A tap on a card opens the block sheet. A pane that asks shows its question
  docked at the bottom (below); one the card cannot answer shows "claude is asking: … Answer" under
  the header. While a full-screen program runs (the alternate screen: vim, htop, less) the pane
  shows its screen ("Keys go straight to htop. When it quits, the pane shows blocks again."),
  the keys wrapped under it and "Show keyboard"; when it quits the blocks come back. **Terminal
  view** shows the pane's screen whatever runs (at a prompt the command line stays under the
  screen). It is on to start with for a pane that runs a coding agent (Claude Code, Codex), which
  never uses the alternate screen, and off for a plain shell; the button turns it on or off for
  that pane until the screen closes. Both screens have **Pinch to zoom · double-tap to reset**
  with **−** and **+** and the percentage (60 to 200 %, kept until the app closes). With a
  computer whose Rift has `fit` and a device that may operate, the zoom also reflows the
  program: the view tells the computer how many columns and rows fit at 11 pt times the zoom
  (after the zoom settles, again every 10 s, released when the screen closes), the text is drawn
  at that size, and the program's lines wrap to the phone's width. Without `fit` the text shrinks
  to the program's columns as before.
  Above the command line: the folder chip (path) and the branch chip (with a dot for uncommitted
  changes) then suggestions (the desktop's completions for what is typed and this pane's history,
  feature `complete`; else its recent commands); the folder chip opens the folder, branch and
  changed-files menus. Under them **⌃**, which shows the key row above the chips, and the round
  field with its run button (Return runs too). While a program runs (Claude Code, a build) the field
  talks to it: "Message for claude", a **Send** button, and Return sends the line as typed text
  and then Return as a key, a second request so the program reads it as a message (feature
  `text`; an older Rift is told to update; authentication as for any key that submits; no
  Confirm sheet). While the keyboard is up the key row starts with
  Hide keyboard and ends with **Edit keys**. The keyboard's smart dashes and quotes are undone in
  the command line.
- **Block cards** (the P3 board): a thin border in the state's colour, no left edge.
  Succeeded: the command, "✓ 1.2s" and the first output line; output that is a table
  (`docker ps`, `kubectl get pods`) is a list of rows, with **Raw** / **As list** under it.
  Failed: the command, "✕ 127", the message (the last output line) and the path it names,
  **Run again** (not on a view-only device) and **Copy error** (not for a command that printed
  nothing). Running: a spinner, the command, its clock and the latest two
  lines (log lines with a level, as "api-1 | [info] GET /health 200 3ms", show as source, level,
  message and time), **Stop** (control C; not for a view-only device) and **Notify when done**
  (**Notifies when done** once on).
- **Block sheet** (a tap on a card): the status ("● Running 0:12", "✓ Succeeded", "✕ Failed ·
  exit 1") and when it ran, the folder and branch, the command with a copy button, the output
  ("Output · 12 lines") with **Copy** and **Load earlier output** where the computer offers it
  (500 lines a press), then **Run again** (**Stop** while it runs) and **Edit**, which puts the
  command in the pane's command line (asking first when that holds other text). Run again is off
  while the pane runs a program or the computer is not connected; view-only devices get no buttons.
- **Question card** (the PaneAsking board): an agent's menu on an `operate` device docks at the
  bottom of its pane in place of the command line: "Claude asks" with how long ago, a minimize
  chevron (to a bar, "Claude asks: …  Answer"), the question, the file with +/− counts and its
  changed lines (**Full diff** / **Less**), the first entry as the filled answer ("Yes"), the other
  entries outlined ("Yes, this session", "Yes, always", "No…"), and a **Keys** row (Esc, ⇧Tab, ↑,
  ↓). **No…** opens a field, "Tell Claude what to do instead", with a send button and **Back to
  the answers**. Answers go as the Answer page's do.
- **Chip menus** (a computer whose welcome lists `chips`; otherwise the folder button does
  nothing), as the desktop's: the folder opens the parent and sub-folders (searchable) and picking one
  runs `cd ..` / `cd ./<name>`; the branch chip the local branches, the current one checked,
  and picking one runs `git checkout <name>` (a new name typed in the search: `git checkout -b`);
  the changed-files chip the code review panel's files, staged first, and a file opens its diff,
  read-only. Names from the computer are untrusted: they are shell-quoted as the desktop quotes
  them (`src/chips.ts` `shellQuote`), never run unquoted. The commands run as the command line's
  Run does (the Confirm sheet first with Ask before running), without a "Notify me".
- **Keys above the command line** (Settings › Panes, off by default): whether the pane
  opens with the key row shown (**⌃** switches it; ^C stays on the running card: Stop); under a
  full-screen program the keys always show.
- **Running**: Run sends at once; Settings › Running commands › Ask before running (off by
  default) shows the Confirm sheet first. With Settings › Notifications › My commands (on by
  default) every command run from this phone asks to be told when it finishes; the bell on a
  running block switches that for one command (and on a computer with `notify_off` also
  drops the computer's own wait).
- **Settings** (the gear on Computers; the Settings board), in groups:
  - **Computers**: each paired computer (green dot while connected, with its system and
    connection) opens its page; **Pair a computer**; under them **This phone's name** (computers
    show it in toasts and logs). A computer's page: **Connection**, **System**, **This phone can**
    (Run commands / View only), **Notifications** (From this computer) and **Unpair this
    computer** (a second tap, "Tap again to unpair", does it).
  - **Notifications**: **My commands** (a command you ran from this phone finishes), **Other
    commands on your computers** (Nothing / Failures / Everything), **Only if it ran longer than**
    (shown unless Nothing), **Show the command**, and **While the app is closed ›** (the ntfy
    page, below). Nothing notifies for the pane on screen; questions always notify.
  - **Running commands**: **Ask before running**.
  - **Security**: **Lock with Face ID** (the device's own name for it) and **Confirm commands with
    Face ID**.
  - **Panes**: **Wrap long lines**, **Keys above the command line**, **Choose keys ›**.
  - **About**: the version and update, **Check now** / **Restart now**, **Check automatically**
    and **What's New**.

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
  it exits; no switch is needed (Terminal view pins the screen). A reconnect or re-subscribe keeps the screen on display until
  the next one comes (or for 5 s, after which the program is taken to have ended). The grid is
  monospace rows with the program's colours (null: terminal text on the pane's ground), bold, italic, underline, dim,
  strikethrough and an inverse cursor; the font fits the width between 7 and 14 pt, scaled by the
  pinch zoom (60 to 200 %), a wider screen scrolls sideways. Each row is text for screen readers; a diff re-renders only its rows.
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
- **Keys** (Settings › Panes › Choose keys, or Edit keys at the end of the row): a preview, the row
  (drag the ≡ handle to reorder; screen readers get Move up / Move down), and Add chips from a
  catalogue (`src/controlKeys.ts`), each key a round cap with its icon or label; the default row
  is Interrupt, Esc, Tab, ⌃, ↑, ↓. ⌃, ⌥ and ⇧ (Shift) are sticky for the
  next key, whatever it is (`ctrl+enter`, `alt+left`, `ctrl+[` is esc, `ctrl+@` is ctrl+space);
  on a key the protocol has no name for the modifier is dropped and the plain key sent.
  **Paste** types the phone's clipboard: into the command line at the prompt, into the running
  program otherwise (feature `text`). Two more catalogue keys are actions, not keys: **+tab** opens a new tab in the pane's window and
  **+pane** splits the pane (feature `open`; disabled on an older Rift), both starting in the
  pane's folder; the app then shows the new pane in place of this one.
- **Block actions** (the block sheet, a tap on a card): Run again (through the
  Confirm sheet when Ask before running is on), Edit (into the command line, asking
  first when that holds other text), Copy command, Copy output; Stop (ctrl+c)
  for a running one. A running card also has the Notify when done bell.
- **Earlier output** (`output`): a block whose tail starts later offers, in the block sheet, "Load earlier output",
  500 lines a press and at most 5,000 per block (`src/earlier.ts`); lines the computer's
  scrollback no longer holds are said to be gone. An answer for another pane or block, or
  starting past the range asked for, fails the load.

## Notifications

Settings › Notifications › Other commands on your computers: Nothing, Failures (an exit status
other than 0) or Everything, for commands that ran longer than "Only if it ran longer than"
(10 s, 30 s, 1 min or 5 min; 30 s by default), when you are not looking at that pane. Each
paired computer has its own Notifications switch (Settings › Computers › the computer, and the
pane's ⋯ menu). A "Notify me" (every command run from this phone while My commands is on, which
it is by default, and a running block's bell) always notifies. The app's own
notifications and the push registration follow the same rules (`src/notify.ts`). Settings saved
before Phase 3 move over (`src/stores.ts`).

## Needs you alerts and the lock screen (the P6 board)

- **Alerts**: while the app runs (on screen, or the short while iOS keeps it alive after), a
  question a pane starts asking posts "claude is asking on studio-mac" with the question; not
  for the pane on screen, not for a computer whose notifications are off, not for the questions
  already waiting when it connected (Computers and the pane show those). A menu with a Yes and a No gets the
  buttons **Yes**, **No** and **Open** (category `rift-ask`), anything else **Open** only
  (`rift-ask-open`). Yes and No need the phone unlocked (Face ID), open the app and send the
  menu's first "Yes…" or "No…" once the pane still asks that question (and, with Ask before
  running on, Face ID in the app first); otherwise the question opens.
- **Live Activity** (iOS 16.2+): each "Notify me" wait (a command run from this phone with Settings ›
  Notifications › My commands on, or Notify when done on the running card) shows the command, where
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
off or the computer's switch is off, `failures_only` for Failures. The computer then
alerts through the Expo push service by itself, also while the app runs; for a finished command
its registration covers (a "Notify me" it received, or a long one by `long_ms` and
`failures_only`) the app posts nothing of its own, for any other it notifies itself, and it drops an
alert about the pane on screen. An alert carries the computer's name, the exit status and,
only with "Show the command", the command line; never output. Settings › Notifications › While
the app is closed says which way notifications come. Without a token (no push project in the
build, or notifications not allowed) the app notifies only while it runs, unless ntfy is on.

### ntfy (free, no push setup)

[ntfy](https://ntfy.sh) delivers the same alerts through its own free app, so it works in a
build without Apple's developer program or a push project (`push.ntfy`, protocol § Push; needs
a computer whose welcome lists `push_ntfy`):

1. Install the ntfy app (App Store, or Google Play / F-Droid).
2. In Rift: Settings › Notifications › While the app is closed › turn on **Alerts through ntfy**. The app makes a random
   topic (`rift-` and 22 random characters) on `https://ntfy.sh` and sends it to every computer.
   To self-host, enter your server's address (https only) in the field under the switch; the
   topic stays, but subscribe again in ntfy on the new server.
3. Tap **Subscribe in ntfy**. On Android it opens ntfy on the topic (`ntfy://<host>/<topic>`,
   [ntfy docs › Integrations](https://docs.ntfy.sh/subscribe/phone/)); without ntfy installed it
   says to install it from Google Play or F-Droid first. ntfy for iPhone has no subscribe link,
   so there the button copies the topic: in ntfy tap **+**, paste it as the topic (for a
   self-hosted server also turn on "Use another server" and enter it), then **Subscribe**.
   **Copy topic** copies the topic on either.

With ntfy on, ntfy replaces Expo push for every computer that supports it (the app sends those
no token), so each command alerts once. A computer whose Rift is too old for ntfy keeps Expo
push; the While the app is closed page says "Update Rift on …". **New topic…** asks, then makes a
new topic and sends it to every computer: subscribe to it in ntfy again; computers that get it stop posting
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

Expo push is then unavailable; ntfy works (Settings › Notifications › While the app is closed). `ios/` is generated
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
                 # Computers / Tabs sheet / pane / answer / Live Activity models (phone.test.ts,
                 # computers.test.ts, topology.test.ts, paneView.test.ts), settings text, embedded fonts
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
- `src/chips.ts` — the prompt chips and their menus; `src/now.ts` — the tab rows and their
  states (Computers, Tabs sheet), the pane's folds and swipe target; `src/paneView.ts` — the Panes
  sheet's rows, block-card text, tables, log lines, the question card's context, the terminal
  zoom; `src/ui/settingsText.ts` — Settings' texts; `src/answer.ts` — what an answer sends;
  `src/live.ts` — the Live Activity plan.
- `src/app/` — expo-router screens, in one stack (no tab bar): Computers (`index`, the home page),
  Settings (`settings`), a computer's page (`computer`), While the app is closed
  (`settings-ntfy`), Paste a pairing link (`add`), Pair a computer (`scan`), pairing confirmation
  (`pair`, also the target of `rift-remote://pair` links), Host (`host/[key]`, a computer's tabs
  and bell; opened after pairing, not from Computers), Pane with its command line and full-screen view
  (`host/[key]/pane/[id]`), Answer (`answer`), the Tabs and Panes sheets (`tabs`, `panes`),
  Confirm sheet, block sheet (`block`), chip menus (`chips`, `diff`), Keys (`control-keys`);
  `src/ui/LockGate.tsx` — the lock screen and privacy cover. On the web a sheet is a plain page: it gets a Close button and
  Escape (`src/ui/SheetClose.tsx`), and the folder button's choice of menus is a strip above
  the command line (react-native-web's `Alert` shows nothing).
- `src/ui/palette.ts` — the DESIGN.md tokens and the text/background pairs the app uses, each
  held to 4.5:1 by `test/contrast.test.ts`.
- The device key, paired desktops and settings live in the iOS keychain / Android keystore
  (expo-secure-store); on web they are kept in memory only.
