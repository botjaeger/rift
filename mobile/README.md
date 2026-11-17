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

`pnpm install` applies `patches/decode-uri-component@0.2.2.patch` to the
router's query decoder. It bounds malformed percent decoding while retaining
the CommonJS API and `+` handling required by `query-string` 7; the upstream
0.5 fix uses ESM. `test/deep-link-security.test.ts` checks the router's resolved
dependency, including forgiving Unicode behavior and bounded decoding work.
The `node-forge@1.4.0` patch requires the RSA DigestInfo algorithm sequence
to contain exactly its validated OID and optional NULL parameters. This
backs up Expo's signing tools until an upstream release includes the fix;
`test/signature-security.test.ts` accepts valid encodings and rejects extra
nested elements with disposable RSA fixtures. Neither patch changes the
phone's native update verifier.

Pair from Portals (the first-run page, or **+**): scan the QR code that Rift shows (Settings › Remote access › Pair a
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
The coding agents' logos (Claude Code, Codex, OpenCode) are Lobe Icons 1.102.0 (MIT,
`assets/agents/LICENSE`), rendered to PNG at 1x, 2x and 3x for 16 and 20 pt (`src/ui/AgentLogo.tsx`;
images, so no SVG library and no native change); OpenCode's one-colour mark takes the accent.

**Portals** is the home page; there is no tab bar. Its gear opens **Settings**, a pushed page;
its **+** pairs a computer. `src/now.ts` and `src/paneView.ts` hold the models.

- **Answer** (the P2 Reply board): "claude is asking", where, **Open pane**; the question, the
  agent's lines above it (a diff drawn as one), then the answers: arrow keys from the selected entry and Enter
  (`src/answer.ts`); the last entry when it is "No" is a field: empty it sends the No, with
  text it sends the No, waits 0.6 s, then the text and Enter ("No, and tell claude what to do").
  A password is a secure field sent as `text` and Enter, never shown, logged or kept. An answer
  goes only to the question it was shown for (the pane must still ask it). View-only devices
  get no answers. Ask before running asks for Face ID first; otherwise nothing confirms. The page
opens from the **No** answer on Portals, from the line under a pane's header for a question its
card cannot answer (a password, other input, a view-only device), and from an alert.
- **Portals** (home, the P4 board): the title with a gear (**Settings**) and **+** (**Pair a
  computer**). No search bar. Under the title one line counts the tabs of every computer in words,
  what needs you first and in orange: "● 1 needs you · 2 working · 1 failed · 27 idle" (counts,
  not filters: the Tabs page filters; `now.ts` `homeSummary`). Below, a card per paired computer:
  a dot (green while connected, blue while trying, red when it cannot be reached), the name,
  "view only" when this phone cannot run commands, and a line that says where it is: "This
  network · 30 tabs", "Over the relay · 1 tab", "Looking on this network…", "Not on this network ·
  trying the relay…" (with a thin progress bar), "Can’t reach it · retry 2 of 5 in 8 s" (counting
  down; grey, its fixes are the rows under it), "Can’t reach it · last seen 2 h ago"
  once it stopped trying (this run's last connection, else the stored `seenAt` of its last
  `welcome`), "Offline · this phone has no network", "Not connected". Two pairings with one name
  (an old one left behind) each add when they were paired ("· paired 3 Nov"). The line is a
  button: it opens the **Connection** sheet (below). On the card's right **New tab** (a 34 pt round
  glass button; on every card of a computer this phone may operate — a connected one only when it
  offers `open`, or `start` while Rift is closed; on one it cannot reach the sheet says it is
  offline). A card that cannot be reached offers its fixes as rows in the card: **Try again** (it
  connects at once, backoff reset) or **Pair again** when it refused this phone's key (other
  failures say why under the name, in full), and **Unpair** in red, which asks first (as the
  card's swipe left does). A connected card shows up to five of its tabs, always in the computer's
  tab order (window by window, then each tab strip's order; a tab never moves when its state
  changes); past five, the five that matter most: what needs you (tinted), then what works, what
  failed and what idles (`now.ts` `homeRows`); **Show all N tabs** opens the Tabs page for the rest. A row: a 36 pt tile
  (the coding agent's logo — Claude Code's on an orange tint, Codex, OpenCode —, `>_` for a
  shell), the tab's name (cut once at its end; a split tab's
  "· 2 panes" stays whole, in the name's white), and under it what its lead pane does, in Inter
  (the tile names the program): "Waiting for you · Make this edit?", "npm run dev · running",
  "cargo test · failed with 101", "Idle · last: echo one", "Idle · ~/code" before any command.
  On the right the tab's 8 pt dot in its state's colour (the accent while it works) and a short
  time beside it in JetBrains Mono ("2m", "3:56", "✕ 1", "12m"; the idle time only for an agent
  the app saw working); a needs-you row is on the accent tint; the state in words
  ("Needs you · 2m", "Working 3:56") is the row's screen-reader label. A tap opens the tab's pane. With one computer paired, a needs-you tab whose agent asks a menu of up to
  three distinct entries answers in place: the question and pills (**Yes**, **Always**, **No**; the
  first entry is filled), and **Open**. A pill sends that entry (Face ID first when Settings asks);
  **No** opens the Answer page with its field. With none paired it is the first-run page (**Scan
  the code**, **Paste a pairing link**). Settings › Computers › a computer opens its page.
- **Connection** (a sheet, from Portals' status line and from the pane's status strip, one
  component for both: `src/ui/ConnectionSheet.tsx`): the status line again, then one row per path
  the computer can be reached by, in trying order: **This network** (a LAN address each) and
  **Relay** (its host), each with one line of where and why ("192.168.1.15 · in use · 6 ms", the
  port left out) and a ✓ on the one in use; the note: "In use · 6 ms" (the round trip of the latest ping; no number until one is answered),
  "No answer on this network (2 s)", "Relay up · your computer has not joined", "The relay
  answered, but your computer is not registered there", or the error the socket reported;
  while connected, **Connection speed** (the last round trip; opens the page below);
  then **End-to-end encrypted** ("The relay passes bytes it cannot read.") and, at the sheet's
  foot, **Test again** while
  connected (it connects again at once, this network's addresses first, so a phone back on the
  home Wi-Fi leaves the relay), or **Try again now** when the computer cannot be reached. The client records each path's state in `ConnState.paths`
  (`src/protocol/client.ts`); the wording is `src/ui/connStatus.ts` (tested in node).
- **Connection speed** (a full page over the Connection sheet; `src/app/speed.tsx`): the median
  round trip in large type with a verdict (*Fast* under 80 ms, *Fair* under 200 ms, else *Slow*);
  **Round trip** (last, median, slowest 5 %, range, jitter, samples: a ping every second while
  the page is open, back to the 15 s keep-alive when it closes); **Traffic** (down, up and screen
  updates per second; bytes received and sent since the connection began, encrypted as on the
  wire); **Typing to screen** (from what is typed, sent or tapped in the pane to its next screen
  update) and **Scrolling to screen** (from a scroll to the next update), each timed as the pane
  is used, only from an input after a second without any (inside a burst the next update may
  answer an earlier input), a wait no update answers within 5 s dropped. The client's `SpeedMeter`
  (`src/protocol/speed.ts`, tested in node) counts; a reconnect starts the numbers over.
- **Tabs** (a full page, from the middle of the pane's tab row; `src/app/tabs.tsx`): every tab of
  that computer, pinned ones first under **Pinned** as on its tab bar, then the rest in the
  computer's tab order. The header (the page header): ‹ Back, "Tabs", the computer and its tab
  count, and **+** New tab (operating devices only; Split pane is in the pane's ⋯). A search field
  ("Search tabs") matches a tab's name, what its lead pane does and any pane's folder; filter
  chips: All, Needs you, Working, Failed, each with its count. A 62 pt row (`src/ui/ListRow.tsx`):
  the grip (on the whole list, a lone pinned tab's too), the program's tile as on Portals (tabs in
  one folder share a name), the tab's name (a pin on a pinned tab,
  "N panes" on a split one, which opens its Panes page), what its lead pane does (Inter), and on
  the right its dot and short time as on Portals, then ✕; the open tab is tinted. A tap goes to that tab's pane in place of this one
  (Back still leads where it led; opened from Portals' **Show all**, the pane opens over Portals);
  ✕ closes the tab. The whole list's captions are **Pinned** and **Tabs · Drag ≡ to reorder**; a
  search or filter lists what it found under "13 tabs" without grips, and says under the rows that
  reordering works on the whole list. A filter that holds nothing says so ("Nothing needs you ·
  Agents that ask a question show here."), centred, with **Show all tabs**. The list is virtualized, so a
  computer of a hundred tabs draws the rows in view only.
- **Panes** (a full page, from the pane row or a tab's "N panes"): the same header (‹ Back,
  "Panes", the tab and its pane count, **+** for the **New pane** sheet) over the same rows, each
  with its program's tile (the agent's logo or `>_`), name, what it does, its mark and short time; the open one
  is tinted. A tap opens that pane in place of this one.
- **The standard** (the Standard board; one component per job in `src/ui/`): `PageHeader`
  (`Screen.tsx`: ‹ Back, the camera page too; title; subtitle; at most two actions),
  `SheetHeader` (the boards' 36 × 5 grabber on every platform — the system's is off —, title,
  subtitle, round ✕ named Close; Escape closes a sheet on the web), `SheetFrame` (every board
  sheet: the header, its content scrolling, its actions pinned to its foot; the sheet is sized to
  its content, whose height is the board's: Attach and What's new 484 pt, Change folder and New
  tab 645, Block 690, New pane 507, Run this? 381, Connection 541 — `src/ui/sheet.ts`; on Android
  the sheet is attached edge to edge with its top corners rounded, on iOS 27 the system floats a
  partial-height sheet inset from the edges, which no sheet option changes), `Field` (46 pt, radius 10, icon, its rim in the accent while it has
  focus, a round clear mark, Inter), `FilterChip` (32 pt, outlined in `rim`), `Button`
  (`primary`, `secondary` outlined in `rim`, `text`, `destructive`; `regular` 48 pt in Inter 600
  16, or `small` 34 pt in 600 13 with a 44 pt hit area; radius 10), round glass header buttons of
  38 pt, the settings rows of `Rows.tsx` (68 pt with or without a line; in cards on settings
  pages, which sit on the page ground under the header's black) with the boards' drawn switch
  (`Toggle`: 51 × 31, a round 27 pt knob; the pane menu's toggles too) and `ListRow` (60 pt
  content rows, edge to edge). A selection, in a block or on a program's screen, is the theme's
  `selection` colour.
  Radii: keys 6, buttons and fields 10, cards 14, sheets 20 (`theme.ts` `radius`). Destructive
  actions are the error red; orange means "needs you" only. Mono is for commands, paths, output
  and keys; a row's line runs through `CodeText` (`codeSpans` in `src/ui/format.ts`).
- **New tab** and **New pane** (sheets: `new-tab`, `new-pane`; opened from the pane's ⋯, the
  Tabs page, the Panes page, and a computer's card on Portals for the tab): **Start with** Shell, Claude Code,
  Codex or OpenCode; New tab first shows **Folder ›**, where it starts (the folder of the tab it
  opens beside), which opens Change folder for that tab (its cd moves the start too). A shell is one `open` (kind `tab` or `pane`; the pane opens in the folder of
  the one the phone is on, with no direction to choose). A program is `open`, then `run` of its
  name in the new pane, tried again for up to 4 s while the new shell has no prompt yet (the
  computer answers `busy` until then); the phone then goes to the new pane, which takes the place
  of the one it was on. `run` follows Settings › Ask before running exactly as any Run does: with
  it on, the sheet first shows the program's name (the Confirm sheet's wording, Cancel and Run at its foot) and Run goes on
  to Face ID if that is on too; a shell types nothing and asks nothing. If the pane opens and the
  program does not start, the sheet says why and offers **Go to the new pane**. A program that is
  not installed is the new shell's own error, which the pane shows. With Rift closed the sheet is
  the **Open a tab** step (Face ID, then `start`) and offers a shell only: the computer keeps no
  program to start for a phone while Rift is closed. View-only devices get no New tab or New pane
  button, and the sheets say so if reached. Logic: `src/launch.ts`; the screens `ui/NewSheet.tsx`,
  `ui/useLaunch.ts`.
- **Open a tab while Rift is closed** (gateways with `start`, which the computer offers only when
  its user turned on *Phones can open tabs while Rift is closed*): with no Rift window open,
  **New tab** (a computer's card, the pane's ⋯, or the Tabs page) asks Face ID (or the passcode) first, then
  opens a shell the computer keeps and lists under "Rift is closed · Tabs" until Rift opens and
  shows it as a tab. View-only devices do not get the button.
- **Reopening**: a cold start goes back to the pane that was open last (`rift.last` in the
  keychain); otherwise Portals.
- **Pane** (the P3 board): commands run from its command line; there is no separate Run page.
  The header: Back (its badge counts the computer's other panes that need you); the computer's
  dot and name with a chevron, which opens the **computer menu** (`src/ui/ComputerMenu.tsx`: every
  paired computer with its state, the current one ticked; a tap on another goes to its last pane,
  one that cannot be reached goes to Portals; then **Connection** and **All portals**);
  **Search output** (a round glass button; its field and "2 of 14 blocks" under it join the header
  block; it filters the blocks by command or output, and over a full-screen program shows the
  blocks until it closes; results show their matching lines without Run again, Copy error or Stop;
  each match in the shown commands and output is marked on the `match` tint, the boards' gold
  `#ffd70055`, at most `MAX_MARKS` per text, `markParts` in `src/paneView.ts`,
  `src/ui/Marked.tsx`; marked output stays a selectable field, its text as styled pieces); and
  **⋯**, a second round glass button. The header block (the New Blocks board: the computer's row,
  the tab row and the search field on the chrome, one hairline under all of it) holds the **tab
  row** (`src/ui/TabBar.tsx`): ‹ and › (plain grey chevrons fixed at the row's ends, 32 pt, no round ground: Back is the header's only round ‹) go to the
  previous and next tab, the new pane sliding in from the side it comes from (a fling across the
  row does the same); the middle, the tab's status dot (grey while the pane opens), its name with
  ⌄ and "tab 2 of 30", opens the Tabs page.
  For a tab of several panes the pane row follows: the pane's mark, name and "· 1/2", a dot and
  count for the tab's other panes that need you, which opens the tab's Panes page. On the Tabs and
  Panes pages a row's grip drags it at once (a long press is not needed; the rest of the row stays
  a button, and the list still scrolls) and dropping it reorders it on the desktop (`move`,
  offered when the computer serves the `move` feature and the device may operate, on the whole
  list only, not a filtered or searched one; a tab takes the place of the tab dropped on; screen
  readers get Move up / Move down actions; `src/ui/ReorderRow.tsx`, maths in `src/reorder.ts`).
  There is no view switch: the pane shows a TUI (its screen) or blocks by one rule, see
  Full-screen programs. A sideways swipe across the blocks (or the picker's Next pane / Previous
  pane screen-reader actions) also moves to the tab's next or previous pane. **⋯** opens a 270 pt
  menu over a scrim, one hairline between every row: two switches, **Wrap long lines** for this
  pane (Settings › Panes › Wrap long lines sets the default; off, each block's output scrolls
  sideways with a position bar) and **Notifications** for this computer, then **Split pane** and
  **New tab** (their sheets; operating devices on a computer that offers them) and Close pane /
  Close tab. Over a full-screen program Wrap long lines gives way to **Text size**
  (`src/ui/TextSizeMenu.tsx`): a tap shows − / the size / + and **Reset**, which set the same
  kept zoom a pinch sets (`setKeptZoom` in `src/paneView.ts`; the screen follows at once), one
  adjustable control for a screen reader. Toasts ("Notifications off for this computer") are a
  dark pill over the tab row. A pane that closed keeps the header and its tab's row: "This pane
  has closed", where, and **Other pane** (the tab's first other pane) and **Portals**.
  **While a pane opens** (the PaneLoading board; `src/ui/paneLoad.ts`, `src/ui/PaneLoading.tsx`):
  the header keeps the tab's last known title (`rift.paneTitles`, written from every topology;
  the computer's name when the pane was never seen). Until the computer is connected a
  full-width status strip sits under it, its text wrapping and never cut ("Connecting through the
  relay" with the wait "0:03", "Can’t reach your computer · retrying in 8 s"); a tap opens the
  Connection sheet. Over skeleton blocks (skeleton rows on the screen view, whose zoom bar waits
  for a screen) a card ("Opening project1") lists the steps as ✓ / ◌ / ○: connected, "Reached
  studio-mac on this network", "Loading the pane’s blocks…" ("Loading the screen…" while the TUI
  shows) and "The screen", with **Details** (the Connection sheet); before that **Reach** the
  computer, **Open this pane**, **Load its blocks**, the current one saying what it waits for. After
  15 s the card is "Taking longer than usual" with **Try again** and **Details** (the sheet); when
  the computer cannot be reached it is "Can’t open this pane" with the reason in full and
  Try again. A pane whose blocks are already held (a reconnect) shows them dimmed under "Reconnecting ·
  showing blocks from 2 min ago". The command field is off, "Waiting for your computer…" ("Waiting…" where that does not fit on one line), until
  the pane's blocks (or screen) have arrived once. The step "Load its blocks" shows no count: the
  gateway does not say how many blocks a snapshot holds.
  Blocks are cards (below); the successes before the newest failure (or the running command) fold
  into "✓ N earlier commands succeeded · Show", and a pane with older blocks than loaded offers
  **Earlier blocks**. A tap on a card's header opens the block sheet; the output under it is selectable text and selecting copies it (see below). A pane that asks shows its question
  docked at the bottom (below); one the card cannot answer shows "claude is asking: … Answer" under
  the header. While a full-screen program runs (the alternate screen: vim, htop, less) the pane
  shows its screen ("Keys go straight to htop. When it quits, the pane shows blocks again."),
  the keys wrapped under it and "Show keyboard"; when it quits the blocks come back. **Terminal
  view** shows the pane's screen whatever runs (at a prompt the command line stays under the
  screen). It is on to start with for a pane that runs a coding agent (Claude Code, Codex: the
  desktop's `running.agent`, its session title, or the program's name), and off for a plain
  shell; the button turns it on or off for that pane until the screen closes. A coding agent's
  pane always has the message input ("Message for claude") under its screen, on either screen
  (Claude Code picks its renderer per launch: the fullscreen one draws on the alternate screen,
  the classic one does not); its **⌃** shows the keys. Other full-screen programs keep the
  keys. On the primary screen with feature `history` (Claude Code's classic renderer, a shell) the
  view is one tall screen: the lines above it come from the computer a page at a time
  (`src/history.ts`; every line numbered, so a line keeps its place while the scrollback is
  trimmed; at most 5000 above the screen), drawn as one light text per line in chunks of 64 that
  stay drawn (the rows near the view first, the rest in the background, so a fling finds them
  there), and the platform's own scrolling and momentum
  move it; a drag sends the computer nothing, and a finger on the view stops it following new
  output until the drag ends at the bottom. Otherwise a vertical drag on the screen scrolls the pane on the computer (feature `scroll`, as the
  desktop's wheel: the agent's or vim's own scrolling, less's lines, or the shell's scrollback),
  counted in rows and sent at most every 30 ms (every 80 ms to a desktop that refuses that pace
  with `rate`, before 0.22.5; `src/scroll.ts`). A drag starts moving as soon as it clears the tap
  threshold. A flick carries on after the finger lifts, slowing as a native list does. Touching
  again stops a flick (a plain drag's last lines still go out); cancelling the gesture, leaving
  the pane or backgrounding the app stops pending motion;
  reversing direction brakes the old flick. A drag or a tap that stops a flick does not open the
  keyboard; a plain tap still does. The computer sends the program's redraws
  every 16 ms while it scrolls, and the app shows each within a frame. Zoomed in, the view scrolls
  itself first and the computer's pane past its edges. A view-only device scrolls a pane's view
  but not a program. With a computer whose Rift has `stitch` (and a device that may operate), a
  full-screen program scrolls natively instead (`src/stitch.ts`): the first vertical drag asks the
  computer for a page; it scrolls the program itself, joins its screens and sends the rows (a
  third of a second to a second for a few screens of Claude Code). The rows above the program's
  scrolling area and below it (a title, an input box, a status line) stay where they are; the rows
  between scroll as a native list with its momentum, the next rows asked for a screen before
  either edge, and half a second after scrolling stops the computer moves the program there, so
  keys and its screen carry on where the view is. New output where the program is ends the page
  (the live screen comes back); a program whose screen does not scroll like a document (htop, a
  menu) answers `flat` and keeps the wheel. While a page shows, a hold selects nothing and a tap
  clicks nothing. **Paste** puts the clipboard in the message box and sends nothing (below); on **Send** an image
  goes to the computer (feature `upload`, operate only, at most 10 MB, `src/upload.ts`), which saves it and
  pastes its path into the running program, as dropping the file on the window does: Claude
  Code and Codex attach it. A file on the clipboard goes the same way (feature `upload_files`): a Files copy through iOS's paste button, the clip's content URI on Android, which shows its own "pasted from" notice. It keeps only its extension (1 to 8 letters or digits; a file without one is refused, and so is anything over 10 MB, at a shell prompt, or to an older computer's Rift, each with a message). On iOS the key block's Paste reads the clipboard itself, so iOS asks "Allow Paste"; the Attach sheet's Paste row is the system's own button, with no prompt. `src/pickFile.ts` wraps `expo-document-picker` for picking a file. Selecting copies, with no Copy button: hold a finger still (450 ms) on the screen and the word under it is selected and copied (a light vibration), with an 18 pt accent knob with a dark rim on each end (the first cell's top left corner, the last cell's bottom right, taken within 44 pt); a dragged handle shows a loupe over the finger (150 × 46 on the surface, the row in mono 22 with a caret) and ticks at each word it steps onto, and lifting it copies the new range (nothing is copied while it moves). The bar docked over the keys shows the lines and characters selected and **✓ Copied** while the selection is on the clipboard (announced once per copy), then **Word**, **Line**, **All** (widen to whole words, whole rows or the whole screen), **To prompt** (disabled unless the command line shows under the screen; a screen reader hears why) and ✕, over "All selects the whole screen."; Android's Back ends the selection; a tap copies nothing and leaves the clipboard as it was; the screen stands still until then (a screen reader's **Copy the screen** action takes it all). A tap is a click for a program that tracks the mouse (feature `mouse`). The message box grows to five lines: **+** beside it opens Attach (what it brings goes in the message, as for Paste below), the phone keyboard's Return is a new line and only the round **Send** or **Run** button sends (an empty box sends nothing; the key row's **Return** key is a new line with text in the box, and with an empty box confirms what the program shows). With a hardware keyboard Shift+Return is a new line and Return sends. **New line** is not in the default key row; it stays in Choose keys. Both screens have pinch to zoom, double-tap to reset, with the controls floating over the screen while it changes
  with **−** and **+** and the percentage (60 to 200 %, kept until the app closes). With a
  computer whose Rift has `fit` and a device that may operate, the zoom also reflows the
  program: the view tells the computer how many columns and rows fit at 11 pt times the zoom
  (after the zoom settles, again every 10 s, released when the screen closes), the text is drawn
  at that size, and the program's lines wrap to the phone's width. While the computer is in use its
  pane keeps its size: a full-screen program is captured at the phone's size whenever it pauses, a
  capture its output moved on from says **Updated N s ago** and gives way after 10 s to the live
  frame at the computer's size, as a program on the primary screen always has; a tap on a capture
  does nothing. Without `fit` the text shrinks to the program's columns as before.
  The command line (the command line redesign board), top to bottom. The context: the prompt
  chips (feature `prompt_chips`: what the desktop draws for the pane, in its order, each chip's
  short value in the desktop's colours, 28 pt pills; one row that never wraps or scrolls, the chips
  that fit by their measured widths and the rest folded into **+N** with a dot for each folded chip
  that needs attention; **+N** opens the *This pane* sheet, `pane-chips`, listing every chip with
  its full value; a tap opens the chip's menu, Change folder or Switch branch). The folder chip
  shows the whole path ("~/code/project1"). A timer (Time on tab, the date or clock) is plain
  text after the chips (Time on tab as "· 2m on tab"), still opening its card. The last chip
  that fits only in part ends in …; the changes chip is cut to its file count ("± 2") instead of
  a count cut mid-number. A
  desktop without the feature gets the folder, branch and changed-files chips from `cwd` and `git`.
  Then the suggestions while typing (the desktop's completions for what is typed and
  this pane's history, feature `complete`; else its recent commands), the first marked in the
  accent: at a prompt the phone keyboard's Return takes it (`src/chips.ts` `returnTakes`).
  Then one line: **+** (the **Attach** sheet), **⌃**, which shows the keys under the field (⌄ while they show), and
  the round field (JetBrains Mono 15) with its 34 pt run button; empty, it says "Command". Then
  the keys (below): labels in JetBrains Mono Bold 14, Paste as the paste icon. Paste
  and Photo are not in the field any more. Otherwise the phone keyboard's Return is a new line in the field, so only the button
  runs; a hardware keyboard's Return runs and Shift+Return is a new line (told apart by the key event
  in modules/rift-keys). The gateway refuses a `run` with a line break (`bad_request`, nothing runs),
  so a command with line breaks says "A command runs as one line" under the field and Run stays off.
  While a program runs (Claude Code, a build) the field talks to the program and says who it
  types to: "Message for claude" ("Message" where that does not fit; ⌃C is ⌃ then C), a **Send** button (all lines go as one message), and Send sends the line as typed text
  and then Return as a key, a second request so the program reads it as a message (feature
  `text`; an older Rift is told to update; authentication as for any key that submits; no
  Confirm sheet). The keyboard's smart dashes and quotes are undone in
  the command line.
- **Block cards** (the New Blocks board): 12 pt corners, a thin border in the state's colour, the
  command on one line and its status as plain JetBrains Mono in the state's colour, output in
  JetBrains Mono 12/17.
  Succeeded: the command, "✓ 1.2s" and the first output line (under Search output, the first line
  that matched), then "299 more lines" when there are more; output that is a table
  (`docker ps`, `kubectl get pods`) is a list of rows, with **Raw** / **As list** under it.
  Failed: the command, "✕ 127", the message (the last output line) and the path it names,
  **Run again** (not on a view-only device) and **Copy error** (not for a command that printed
  nothing). Running: a spinner, the command, its clock and the latest
  line (a log line with a level, as "api-1 | [info] GET /health 200 3ms", shows as source, level,
  message and time), **Stop** (control C; not for a view-only device) and **Notify when done**
  (**Notifies when done** once on).
- **Block sheet** (a tap on a card's header): the command as the title, then the status and how long
  ("Failed with 1 · 2.4 s"; "started 2 min ago" while it runs), the folder (cut at its start) and branch, the command with a copy button, the output
  ("OUTPUT · 12 LINES", in a 150 pt box; a longer output takes the room down to the sheet's foot and scrolls inside it, the rest of the sheet staying put; "(no output)" is one line; selecting any of it copies the selection) with **Copy** for all of it and **Load earlier output** where the computer offers it
  (500 lines a press), then, pinned to the sheet's foot, **Run again** (**Stop** while it runs) and **Edit**, which puts the
  command in the pane's command line (asking first when that holds other text). Run again is off
  while the pane runs a program or the computer is not connected; view-only devices get no buttons.
- **Question card** (the New Screen board): an agent's menu on an `operate` device docks at the
  bottom of its pane in place of the command line, where the keys are, never over the screen,
  598 pt tall on an 874 pt phone (shorter phones in proportion), with a neutral rim along its top:
  the standard sheet header: "Claude Code asks" (the program by its own name) over the tab and how
  long ago, and a round ✕ that folds it (to a 48 pt bar in a black strip under a hairline, "Claude
  Code asks: …  Answer"), the question (19/25), the file in mono 12 with +N in green and −N in red
  and its changed lines (30 pt rows on their tints, the line number in an 18 pt column), the first
  entry as the filled answer ("Yes"), the other entries outlined in equal halves, each saying what
  it does ("Yes, this session", "Yes, always", "No, explain"), and a **Keys** row (Esc, ⇧Tab, ↑, ↓,
  30 pt keys in mono 12). **Full diff** opens **A change in full** (`app/change.tsx`), a page of
  its own: the file, "+3 −1 · from Claude Code", the hunk header strip (the agent's own, else
  made from the first line number and the counts) and every line of the change, kept ones too,
  edge to edge on their tints; it keeps the question as opened, so an answer meanwhile leaves it
  readable until Back. **No, explain** turns the card into **Tell it why**: a sheet header
  ("Claude Code · project4", ✕ back to the answers) over a 120 pt reply box (its lines wrap;
  Return sends) and a full-width **Send**. Answers go as the Answer page's do.
- **Pull request, Python, PHP, Docker and Time on tab menus** (feature `prompt_chips`): the pull
  request opens its URL in the browser (`https` only); Python and PHP list the installed versions
  and a pick runs the desktop's own `rift-use …`; Docker lists containers (ports, uptime, health)
  and a pick follows the service's logs; Time on tab shows the desktop's line and **Reset**
  (operate only). A pick runs as the command line's Run does (the Confirm sheet first with Ask
  before running); only the two command shapes these menus build are accepted
  (`src/chips.ts` `isMenuCommand`). Logic: `src/promptChips.ts`; the row: `src/ui/ChipRow.tsx`.
- **Chip menus** (a computer whose welcome lists `chips`; otherwise the folder, branch and changes
  chips do nothing), as the desktop's: the folder opens the parent and sub-folders (searchable) and picking one
  runs `cd ..` / `cd ./<name>`; the branch chip the local branches, the current one checked,
  and picking one runs `git checkout <name>` (a new name typed in the search: `git checkout -b`);
  the changed-files chip the code review panel's files, staged first, and a file opens its diff,
  read-only. Names from the computer are untrusted: they are shell-quoted as the desktop quotes
  them (`src/chips.ts` `shellQuote`), never run unquoted. The commands run as the command line's
  Run does (the Confirm sheet first with Ask before running), without a "Notify me".
- **Keys under the command line** (Settings › Panes, off by default): whether the pane
  opens with the keys shown (**⌃** switches them); under a
  full-screen program the keys always show.
- **Running**: Run sends at once; Settings › Running commands › Ask before running (off by
  default) shows the Confirm sheet first ("Run this?", "Ask before running is on", the command,
  "In ~/code/project1 on studio-mac.", Cancel and Run at the sheet's foot). With Settings › Notifications ›
  Commands I ran from here (on by
  default) every command run from this phone asks to be told when it finishes; the bell on a
  running block switches that for one command (and on a computer with `notify_off` also
  drops the computer's own wait).
- **Settings** (the gear on Portals; the Settings board), in groups sorted by what people look for:
  - **Computers**: each paired computer (Portals' dot, and where it is: "This network · can run
    commands", "Over the relay · view only", "Can’t reach it") opens its page; **Pair a computer**.
    A computer's page: its system and pairing day under the name ("macOS · paired 3 Nov"), then
    **This computer**: **Connection** (This network, Over the relay, Can’t reach it; opens the
    Connection sheet, as the computer menu does), **Access**
    (Can run commands / View only), **Theme** (the theme it sends, "Dracula"; opens Appearance),
    **Notifications** (finished, failed and questions), and **Unpair this computer**, an outlined
    red button with what it means under it (a second tap, "Tap again to unpair", does it).
  - **This phone**: **Name ›** (computers show it in toasts and logs; a tap edits it in place), **App lock** (Face ID, the
    device's own name for it; on opening and after 5 minutes away) and **Appearance ›** (the Themes board; `src/app/appearance.tsx`): **Same as the computer** (the
    default: the theme of the computer in view, "studio-mac uses Dracula now"), **Match this
    phone** (Light Default by day, Dark Default at night) or **Choose a theme ›**, then
    the desktop's 13 builtin themes as a five-column grid of swatches (ground and accent dot; a
    tap chooses one, ringed in the accent). The choice stays on the phone and never changes a computer.
  - **Notifications**: **Commands I ran from here** (a command you ran from this phone finishes),
    **Other commands ›** (Nothing / Failures / Everything, opening under it), **Only if it ran longer than**
    (shown unless Nothing), **Show the command**, and **While the app is closed ›** (the ntfy
    page, below). Nothing notifies for the pane on screen; questions always notify.
  - **Running commands**: **Ask before running** and **Confirm with Face ID** (needs App lock and
    Ask before running).
  - **Panes**: **Wrap long lines**, **Keys under the command line**, **Keys ›** (its keys beside it).
  - **About**: the version and update, **Check now** / **Restart now**, **Check automatically**
    and **What's new**.

The lock screen shows Rift's mark, "Rift is locked", "Your tabs stay connected." and **Unlock
with Face ID** (the device's own name for it).

## Connecting

- Order: the path that last worked, else each LAN address (1.5 s to connect and finish the
  handshake while a relay is there to fall back on, 8 s without), then the relay
  (`<r>/connect?id=<routing id>`). Host status and the pane banner name the path
  ("LAN 192.168.1.29", "Relay relay.example.com"). A LAN address that answers with
  another key than the paired one is skipped; the host stops with "key mismatch" only when the
  relay, or every path, does. A network change tries it again.
- The computer's addresses follow it: each `welcome` names where it can be reached now (it moves
  between networks, and the pairing code's addresses go stale), stored first with the host, the
  older ones after (4 in all). Over the relay, each new one is opened beside it; the first that
  answers takes the connection over (the pane resumes on it), and none that does leaves the relay
  as it was.
- A command, key or answer on its way when the connection drops or the wait runs out is never sent
  again by the app (a reconnect carries none of it; an old acknowledgement settles nothing new).
  The error says "It may have gone through: check the pane before sending it again", and the draft
  stays. A resumed pane view whose position belongs to an earlier gateway run gets a fresh snapshot.
- After the first failed connection round, the app tries up to five reconnect rounds,
  with jittered exponential backoff (0.5 s base, capped at 10 s).
  **Cancel** stops the attempts early; **Try again** starts them again. Coming back online also
  restarts a host that gave up. A desktop update's announced restart has its own retry window
  (see `closing` in [the protocol](../docs/remote-protocol.md)).
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
  it exits. There is no Blocks | Terminal switch: the pane shows the TUI for the alternate screen,
  a coding agent (`runsAgent`, also Claude Code's classic renderer on the primary screen) or a
  program no block holds (reattached, a shell without integration), and blocks for everything
  else, `cargo build` and `tail -f` included (the app asks for mode `on` when the rule, not the
  alternate screen, asks for the TUI). A plain `read -p` or a REPL without the alternate screen
  gets no screen; password and choice prompts still arrive through `waiting`. A gateway without
  `screen` always shows blocks. A reconnect or re-subscribe keeps the screen on display until
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
- **The keys** (the Keys redesign board; `src/ui/ControlRow.tsx`): one strip for the command line
  (shown by **⌃**) and a full-screen program alike, 88 pt high so the program keeps its size and
  the screen does not move: the key block, two rows of 41 pt keys with 8 pt corners (three
  columns for six keys, more when Settings › Keys holds more), the keyboard key always last (Hide
  keyboard while it is up, else Show keyboard), and beside it the 132 x 88 arrow pad, ↑ above ← ↓
  →. A held arrow repeats (after 400 ms, then every 80 ms) until the finger lifts, the touch is
  cancelled, the keys go away or the app leaves the foreground; a screen reader's activation sends it once.
- **Keys** (Settings › Panes › Choose keys; the New Settings
  board): the header says how many of the block's 8 places are used (the keyboard key is one), a
  preview of the key block and arrow pad, the block's keys (drag the ≡ grip to reorder, the shared
  reorder row; − in red removes; screen readers get Move up / Move down / Remove), and Add a key,
  a search field over the catalogue (`src/controlKeys.ts`) and a chip for each key not in the
  block (the arrows are always on the pad and never listed), in the board's order (⇧Tab, ⌥, ⇧,
  Home, End, Page up, Page down, ⌃D ⌃Z ⌃R ⌃L ⌃U, one **F1–F12** chip that lays out the twelve,
  then Interrupt, New line, ⌫, Del, Ins, ⌃A ⌃E ⌃W, New tab, New pane); a search finds each key. The default block is Esc, Tab, ⌃,
  Return, Paste and the keyboard key (New line, Interrupt and the rest are in Add a key). A saved
  list that is any earlier default (Esc, Tab, Interrupt, ⌃, ↑, ↓, Return; Interrupt, Esc, Tab, ⌃,
  ↑, ↓; Esc, Tab, Interrupt, ⌃, ←, ↑, ↓, →; that with New line) moves to this default; a custom
  list keeps every key, its arrows moving to the pad, so nobody loses a key.
  ⌃, ⌥ and ⇧ (Shift) are sticky for the
  next key, whatever it is (`ctrl+enter`, `alt+left`, `ctrl+[` is esc, `ctrl+@` is ctrl+space),
  and stay lit (the accent, dark ink) until it, so ⌃ then C is one chord; a screen reader hears
  them as selected and is told when one is held or let go;
  on a key the protocol has no name for the modifier is dropped and the plain key sent.
  **Paste** types the phone's clipboard: into the command line at the prompt, into the running
  program otherwise (feature `text`); in the message box an image or file on the clipboard waits
  above the field like Attach's (below). Two more catalogue keys are actions, not keys: **+tab** opens a new tab in the pane's window and
  **+pane** opens a pane beside this one (feature `open`; disabled on an older Rift), both starting in the
  pane's folder; the app then shows the new pane in place of this one. Each key draws an 18 pt icon
  from the one set for every symbol (arrows, ⌃ ⌥ ⇧, Return, Paste, New pane, the keyboard; ⇧Tab leads with
  the shift icon) and JetBrains Mono for keys and chords (Esc, ⌃D);
  colour marks a state only (Interrupt's red, a held modifier lit in the accent). Paste is an
  ordinary key with the set's paste icon (user, 2026-11-07), so iOS asks "Allow Paste" when it
  reads the clipboard; the Attach sheet's Paste row stays the system's control, with no prompt.
  The keys never scroll.
- **Attach** (a sheet, from **+** in the message box; `attach`, logic in `src/pending.ts`): **Paste**
  (text, an image or a file from the clipboard), **Photo library**, **Take a photo** and **File**
  (`src/pickFile.ts`; photos by `expo-image-picker`, `src/pickPhoto.ts`), as ordinary rows. On iOS
  the Paste row's title is the system's own paste control (`PasteButton` in modules/rift-keys: the
  system's word "Paste" on the card's ground), so a tap on the word reads the clipboard with no
  "Allow Paste" prompt; elsewhere the row reads the clipboard. Nothing pasted reaches the program before **Send**, by any route (Attach, the key block's Paste, iOS's
  paste control, a keyboard's Paste key). Short text goes into the message box at the cursor. An image, a
  file, or a text of more than 3 lines or over 4 KB goes in as a token at the cursor (`src/draftTokens.ts`):
  `[Image 1]`, `[File note.pdf]`, `[Text 12 lines]`, at most 4 in a box, and the user types around it, as in
  Claude Code. Deleting any part of a token deletes all of it and its item; text typed inside it lands after
  it; the same words typed by hand are plain text. **Send** sends the box in order, each image or file through
  the computer and a long text as one paste, then Return as its own request, also with only tokens in the box.
  It waits 1.1 s after each upload, and for an image to Claude Code until its input box shows one more
  `[Image #N]` (15 s at most, 4 s with the pane's screen out of sight): Claude Code places the image where its
  cursor is once it has read it, 0.9 s for 10 MB on an idle computer and 3 s on a busy one. A long text pasted
  through the field itself (the phone's Paste menu, the keyboard's clipboard, a hardware ⌘V) becomes a token too, and
  a hardware Ctrl+V in the box pastes the phone's clipboard like the key block's Paste. A Send once begun
  finishes without the screen, Return included, and a second one waits for it; during it only words added after
  the draft are taken, for the next message. The first failure stops: what went has left the box, the rest stays
  in it with its items, and the message names what failed and why. The key block has
  Paste where the computer takes images and a program runs (at its end when Settings › Keys lacks it). A full-screen program with its own keys (vim, htop) has no box of its own: its Paste
  opens the pane's message box under the screen until the box is sent or emptied. The items live in memory, as
  drafts do: neither survives a relaunch.
  The sheet returns to the pane it came from with the draft kept; Close, or the sheet's swipe, adds
  nothing. At a shell prompt an image or file is refused with the same message as a Paste
  (there is no program to take it); text still goes in the field. **Take a photo** asks for the
  camera first (iOS refuses the launch without it) and says how to allow it when refused; the usage text (`NSCameraUsageDescription`, set on the
  `expo-camera` and `expo-image-picker` plugins alike) names both uses, scanning the pairing code
  and taking a photo, and the image-picker plugin no longer blocks the Android `CAMERA`
  permission the scanner needs. The full-screen program's key block keeps its Paste, which sends an
  image at once.
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
  already waiting when it connected (Portals and the pane show those). A menu with a Yes and a No gets the
  buttons **Yes**, **No** and **Open** (category `rift-ask`), anything else **Open** only
  (`rift-ask-open`). Yes and No need the phone unlocked (Face ID), open the app and send the
  menu's first "Yes…" or "No…" once the pane still asks that question (and, with Ask before
  running on, Face ID in the app first); otherwise the question opens.
- **Live Activity** (iOS 16.2+): each "Notify me" wait (a command run from this phone with Settings ›
  Notifications › My commands on, or Notify when done on the running card) shows the command, where
  it runs, a clock the system ticks by itself (`Text(timerInterval:)`) and its latest line on the
  lock screen and in the Dynamic Island (where = computer · tab, with the pane's number when the
  tab has several); its `done` turns it into ✓ or ✕ N, which stays 15 minutes. A tap opens that
  pane (`rift://host/<key>/pane/<id>`), where the running card's bell drops the wait; swiping it
  off the lock screen dismisses it. One whose `done` came while the phone was disconnected (it is
  not replayed) ends at the first topology after the reconnect that shows the pane, seen running
  before, back at its prompt: ✓ or ✕ N from the pane's last command when that is the one asked,
  with a "Finished" / "Failed" notification unless the computer already alerted through push. Not
  updated for an hour (the app refreshes it every 20 minutes while it runs), it shows "Open Rift"
  instead of the clock. The widget extension `RiftLive` (`targets/RiftLive`, added by
  `plugins/withLiveActivity.js`) needs no entitlement, so a free Apple ID signs it like the app;
  the app drives it through `modules/rift-live` (`src/live.ts` plans it).
- **The APNs limit**: without Apple's developer program (`RIFT_NO_APNS=1`) there is no APNs, so
  nothing reaches the phone while the app is suspended except through ntfy: no Needs you alert
  (the computers' push and ntfy alerts are about finished commands only), and a Live Activity
  is not ended by a `done` that comes while the app is suspended: it shows "Open Rift" after an
  hour, iOS ends it after 8 hours, and opening the app ends it once the computer answers. With an APNs key the
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
the app is closed says which way notifications come (without a token: "Without it, notifications
come only while this app is open: this phone gave no push token."). Without a token (no push project in the
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

1. An Expo account and an EAS project (`npx expo login`, then create the project on
   expo.dev). Put `EXPO_OWNER=<account>` and `EAS_PROJECT_ID=<id>` in
   `~/.config/rift-release/expo.env` (or the environment): `app.config.js` adds the owner,
   the update URL and `extra.eas.projectId` from them. Without them a build has no
   over-the-air updates and no Expo push.
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
                 # Portals / Tabs page / pane / answer / Live Activity models (phone.test.ts,
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
  pane reducer and the screen reducer (`store.ts`), 100 ms batched store (`batch.ts`; a screen frame publishes within 16 ms).
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
- `src/chips.ts` — the folder, branch and changes chips and their menus; `src/promptChips.ts` — the
  desktop's prompt chips, the row's fit and the new menus' rows; `src/now.ts` — the tab rows and their
  states (Portals, Tabs page), the pane's folds and swipe target; `src/paneView.ts` — the Panes
  page's rows, block-card text, tables, log lines, the question card's context, the terminal
  zoom; `src/ui/settingsText.ts` — Settings' texts; `src/answer.ts` — what an answer sends;
  `src/live.ts` — the Live Activity plan.
- `src/app/` — expo-router screens, in one stack (no tab bar): Portals (`index`, the home page),
  Settings (`settings`), a computer's page (`computer`), While the app is closed
  (`settings-ntfy`), Paste a pairing link (`add`), Pair a computer (`scan`), pairing confirmation
  (`pair`, also the target of `rift-remote://pair` links), Host (`host/[key]`, a computer's tabs
  and bell; a cold start can come back to it. Pairing lands on Portals, on the new computer's card), Pane with its command line and full-screen view
  (`host/[key]/pane/[id]`), Answer (`answer`), the Tabs and Panes pages (`tabs`, `panes`: full-screen modals),
  Confirm sheet, block sheet (`block`), chip menus (`chips`, `diff`), Keys (`control-keys`);
  `src/ui/LockGate.tsx` — the lock screen and privacy cover. On the web a sheet is a plain page: its
  header's ✕ and Escape close it (`src/ui/SheetHeader.tsx`), and the folder button's choice of menus is a strip above
  the command line (react-native-web's `Alert` shows nothing).
- Colours come from a theme at runtime. `src/ui/palette.ts` (`paletteFrom`) turns a desktop
  theme's tokens into the app's palette (the token names one to one, tints and raised greys
  derived from them) and lifts any text colour that falls short of 4.5:1 on a ground it is drawn
  on (`TEXT_ON`, `TINTED_TEXT`; `test/contrast.test.ts` runs every builtin theme; Dark Default
  needs no lift). The root layout provides the palette Settings › Appearance resolves to
  (`src/appearance.ts`, `src/ui/useAppPalette.ts`): with **Same as the computer**, the theme the
  computer in view sent (welcome and `theme`, feature `theme`; the pane's or host page's
  computer, elsewhere the last one opened), else the last one shown, else Dark Default; each
  computer's theme and the last one are kept in the keychain (`rift.theme.<key>`,
  `rift.theme.last`). Components read colours with `useColors()` and make their styles with
  `const useStyles = makeStyles((colors) => ({ … }))` / `const styles = useStyles()`
  (`src/ui/theme.ts`; `fromPalette` for a map of colours by tone); nothing reads a colour at
  module load. The camera page and the splash keep Dark Default. `src/themes.data.ts` is the
  desktop's builtin themes, generated by `RIFT_WRITE_THEMES=1 cargo test -p rift-terminal
  themes_data` (that test fails when the copy is out of date). `app.json`'s
  `userInterfaceStyle` is `automatic` so that **Match this phone** can see the system's light or
  dark (a native build); text fields take the palette's keyboard appearance.
- The device key, paired desktops and settings live in the iOS keychain / Android keystore
  (expo-secure-store); on web they are kept in memory only.
