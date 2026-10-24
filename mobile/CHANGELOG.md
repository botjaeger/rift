# Changelog — Rift for iPhone and Android

User-facing changes of the phone app, newest first. Each section is a native
build (`## <version> — <YYYY-MM-DD>`) or an over-the-air update to one
(`## <version> · update <N> — <YYYY-MM-DD>`); an update's summary is the
message its download carries. Items use `### New`, `### Improved` or
`### Fixed` sections of `- **Title** — text`.

## 0.4.1 · update 2 — 2026-10-25

Scrolling starts the moment you drag and stops when you leave.

### Fixed

- **Scrolling full-screen programs** — a drag moves the program as soon as it starts, and a quick
  drag no longer flicks on its own. Leaving the screen or switching apps stops a flick, and
  dragging or catching a flick no longer opens the keyboard.

## 0.4.1 · update 1 — 2026-10-24

Smoother scrolling in Claude Code and other full-screen programs.

### Improved

- **Scrolling full-screen programs** — dragging follows your finger, a flick keeps scrolling and
  slows down like any list, and a touch stops it. Smoothest with Rift 0.22.5 on the computer.

## 0.4.1 — 2026-10-24

Safer handling of links that open the app.

### Fixed

- **Links that open the app** — a malformed link can no longer stall the app while it is read.

## 0.4.0 — 2026-10-23

Scroll and paste screenshots into Claude Code and other full-screen programs, and agents keep their message box.

### New

- **Scroll full-screen programs** — swipe on Claude Code, Codex, vim or less to scroll them. Your phone
  keeps its own place: scrolling here no longer moves the view on the computer.
- **Paste screenshots** — Paste with an image on the clipboard sends it to the computer, which pastes
  its path into the program, so Claude Code and Codex attach it. Paste is on the key row wherever images
  can go, and a second Paste while one is sending says so.

### Improved

- **Agents keep the message box** — Claude Code, Codex and other agents get the message input even in
  full-screen mode; vim and less keep the key row.
- **Blocks | Terminal** — the switch shows Terminal while a full-screen program fills the pane.
- **Leaving a screen** — something you started on a screen you then left (a run, a save, Unpair) no
  longer finishes on the screen you are on now.

### Fixed

- **Android: denying notifications** — no longer closes the app during a confirmed run.
- **Confirmations** — closing a confirmation goes back to the pane it came from.
- **Settings and phone name** — an edit you were making is no longer lost or overwritten by an older save.
- **Sent text** — text the computer already took no longer stays in the message box.

## 0.3.0 · update 5 — 2026-10-19

Reconnecting stops after 5 tries, panes have a Cancel button, and you can swipe a computer to unpair it.

### New

- **Swipe to unpair** — swipe a computer left on Portals to unpair it.
- **Cancel** — a pane that is reconnecting has a Cancel button that stops trying and goes back to Portals.
- **Rift quit / Updating Rift** — when the computer quits Rift the app says so and waits for you; when
  Rift is updating it says "back in about a minute" and reconnects by itself.

### Improved

- **Reconnecting** — the app tries 5 times, then stops and waits for Try again (coming back to the app
  or the network also tries again).
- **Swiping across results** — side-scrolling a result no longer switches panes; pick a pane from the list.

## 0.3.0 · update 4 — 2026-10-19

Settings and paired computers no longer get overwritten, and a failed save or Unpair tells you and lets you retry.

### Fixed

- **Turning on the app lock** — an answer to the notification prompt arriving afterwards no longer
  turns the lock back off, and a failed older save no longer cancels a newer choice.
- **Paired computers** — saving or removing computers at the same moment no longer loses one.
- **Notification choices that could not be saved** — the app shows the error and keeps the old choice
  instead of showing one that was not saved; the same control can be tried again.
- **Unpair** — when it fails, the button works again and says what to do.

## 0.3.0 · update 3 — 2026-10-18

Tapping an alert opens the right pane, and a send that timed out says to check first.

### Fixed

- **Opening from an alert** — tapping a notification when the app was closed no longer shows "This
  computer is not paired" before your computers have loaded.
- **A send that timed out** — the message now says it may have gone through, so you check the pane
  before sending it again.
- **After your computer restarts Rift** — the app shows the pane again instead of waiting on an old
  position.

## 0.3.0 · update 2 — 2026-10-18

Text you type while a command is sent is kept, and a few safety fixes.

### Fixed

- **Text typed after Run** — what you type while a command is being sent stays in the command line
  when the computer confirms it.
- **Settings that failed to save** — a setting your computer could not save goes back to what it
  was and says so, instead of showing the change as made.
- **Closing the last pane** — the pane's old output and command line go away with it.
- **Tabs and the app lock** — the Tabs sheet is covered by the app lock and hidden when the app is
  in the background, like the rest of the app.

## 0.3.0 · update 1 — 2026-10-16

Your computer's prompt chips in one row, with their menus.

### New

- **Your computer's prompt chips** — the row above the command line shows the chips Rift draws on
  your computer (folder, branch, changes, pull request, Python, PHP, Docker, time on tab and
  more), in your order and colours. It is one row that never scrolls: what does not fit folds into
  +N, with a dot for each folded chip that needs attention (a Docker container that is down),
  and +N opens This pane with every chip. Needs Rift 0.10.0 or later on the computer with
  prompt chips; an older Rift keeps the folder, branch and changes chips.
- **Chip menus** — a pull request opens in your browser; Python and PHP list the versions and
  switch the shell; Docker lists containers and follows a service's logs; time on tab has
  Reset. Commands honour Ask before running.

## 0.3.0 — 2026-10-16

Your computer's look on your phone, closing tabs and panes, an easier key row, and seeing how
your phone reaches your computer.

### New

- **Connection details** — each computer on the home screen says how it is connected (this
  network or the relay) and what it is doing; tap the line to see both routes, which one is in
  use, its delay, and why the other is not. When it can't be reached it says why and offers
  *Try again*.
- **Opening a pane** — while a pane opens you see each step (reaching your computer, opening
  the pane, loading its blocks), how long it has taken, and after 15 seconds what may be wrong.
  The pane keeps its name instead of "Pane", and reconnecting shows its last blocks dimmed
  instead of an empty screen.

- **Close tabs and panes** — close a pane or a tab from the ⋯ menu, or with ✕ in the tab and
  pane lists. When a command is still running, the phone asks first.
- **Key tips** — hold a key to see what it does; tap it to send it.
- **Prompt chips** — the row above the command line shows your folder, branch and changed
  files; while you type it shows completions instead.

### Improved

- **Portals** — the home screen is now called Portals.
- **Same look as your computer** — the terminal view uses your computer's theme colours,
  the bar beside each command and the status badges, and emoji keep the columns aligned.
- **Warp's colours** — the app uses Warp's dark colours, like Rift on your computer.
- **Blocks or Terminal** — a Blocks | Terminal switch replaces the old button and its note;
  a Claude Code or Codex pane shows only its terminal.
- **Keyboard** — the last line stays above the keyboard, and hiding the keyboard is a small
  key at the end of the row.
- **Default keys** — Esc, Tab, ⌃C, ⌃ and the four arrows, all visible without scrolling.
- **Alerts** — notifications and the lock-screen activity show the Rift logo, the result,
  how long it took and where it ran.

### Fixed

- **Wrap long lines** — the setting now shows only where it applies (Blocks).

## 0.2.0 · update 6 — 2026-10-15

Talk to Claude Code from your phone, and its pane opens in the Terminal view.

### New

- **Message a running program** — while Claude Code or another program runs, the command line
  sends what you type to it, then Return.
- **Zoom fits the program** — with Rift 0.7.0 on your computer, zooming the Terminal view
  resizes the pane, so the program lays itself out for your phone.

### Improved

- **Terminal view for agents** — a pane running Claude Code opens in the Terminal view; the
  terminal button still switches back to the commands.

## 0.2.0 · update 5 — 2026-10-15

A new look: Computers is the home page, panes show commands as cards, and Settings is rebuilt.

### New

- **Computers home** — every computer as a card with its connection, its tabs and what each one
  is doing, chips to show only what needs you, works or waits, and New tab on the card. The gear
  opens Settings and + pairs a computer.
- **Tabs and Panes sheets** — tap a pane's title to switch tabs or open a new one; the pane
  picker lists every pane of the tab.
- **Command cards** — each command shows as a card with its state, output laid out for the
  phone, and its log lines while it runs; tap a card for its actions.
- **Question card** — when an agent asks, the question docks above the command line with its
  answers, and folds away until you are ready.
- **Terminal view** — pinch to zoom the full-screen terminal, or use − and +.

### Improved

- **Settings** — a page of its own behind the gear, with each computer on its own page.
- **Fewer places to look** — Now, the tab bar and the Run page are gone; what they showed now
  lives on Computers and in the pane.

## 0.2.0 · update 4 — 2026-10-15

Now for one computer: its connection at the top, chips to show one kind, and idle agents in view.

### Improved

- **One computer on Now** — with one computer paired or picked, Now names it and its
  connection, chips show only what needs you, works or waits, idle agents are listed with how
  long they have waited, and finished commands fold into one row.

## 0.2.0 · update 3 — 2026-10-15

Now handles several computers: pick one or see all, with what waits folded away.

### New

- **Choose computers on Now** — with two or more paired, the pill under Now shows every
  computer or the one you pick; while it mixes them, each avatar carries the computer's
  two-letter badge in its own color.

### Improved

- **A shorter Now** — Working shows three sessions, then "more working"; idle agents and
  finished and failed commands fold into one row you open when you want them.
- **Sessions by name** — Claude Code and Codex sessions show their own name, with the
  folder and branch beneath (needs Rift 0.6.0 on the computer).
- **Round buttons** — Run is a round button, and a question's answers are round pills.

## 0.2.0 · update 2 — 2026-10-15

A fresh look: glass headers, project avatars, search on Computers and rounder keys.

### New

- **Search tabs and commands** — a search bar on Computers finds tabs on every computer
  by title, command, program or folder.
- **Terminal view** — the pane's header shows its screen as it is, whatever runs, until
  you press it again.
- **Paste and Shift keys** — Paste types your clipboard; Shift applies to the next key.
  Add them in Settings › Keys.

### Improved

- **New look** — round glass headers whose back button counts the panes that need you,
  colored avatars for each project, round keys with icons, and a round command line with
  its own run button.
- **Hide keyboard** — the key row starts with a key that puts the keyboard away.

### Fixed

- **What's New closing itself** — after an update, What's New stays open until you close
  it.

## 0.2.0 · update 1 — 2026-10-15

Page titles stay below the clock after you type.

### Fixed

- **Titles under the clock** — after typing in the Run sheet or any field, the Now,
  Computers and Settings titles no longer slide up under the status bar.

## 0.2.0 — 2026-10-15

Updates you can see and control: what changed, when to check, and the version you run.

### New

- **What's New** — after an update, the app shows what changed once; Settings › About ›
  What's New lists the changes of every version.
- **Update checks** — Settings › About › Check now looks for an update on demand, and
  Check for updates automatically turns the automatic checks off or on.
- **Open a tab while Rift is closed** — when your computer allows it, a computer whose Rift
  window is closed offers Open a tab: the shell starts on the computer, and the tab appears
  there when you open Rift.
- **Opening animation** — the app opens on Rift's prompt typing its name, in under a second.

### Improved

- **Version** — Settings › About names the update you run, such as "0.2.0 · update 1".

## 0.1.0 · update 2 — 2026-10-15

The tab bar is always there and steps aside while you scroll; updates arrive while the app is open.

### Improved

- **Tab bar** — shows from the first launch, before a computer is paired; on iPhone it
  shrinks while you scroll down and returns when you scroll up.
- **Updates** — the app also looks for an update while it is open and when you come back to
  it, and offers to restart once one has downloaded.

### Fixed

- **Page ends** — the last item on a page no longer sits far above the tab bar.

## 0.1.0 · update 1 — 2026-10-14

Smoother screen transitions on iPhone.

### Fixed

- **Screen transitions** — going back no longer flashes a white edge
  around the screen.

## 0.1.0 — 2026-10-14

The first release of the Rift companion: follow and drive the terminals on
your computers from your phone, end to end encrypted.

### New

- **Pairing** — scan the QR code under Rift's Settings → Remote access; the
  phone remembers each computer and reconnects by itself, on your network or
  from anywhere through a relay.
- **Now** — what needs you, what is running and what just finished, across
  every paired computer.
- **Live panes** — follow a pane's blocks as they run, open a block's full
  output, and switch to a terminal view for full-screen programs.
- **Run commands** — type or pick a command, with completions and chips for
  folders, branches and recent commands; ⌃C and the other control keys are
  one tap away.
- **Answers** — when a command waits for yes or no, answer from the alert or
  the lock screen.
- **Live Activities** — a command you asked to watch shows its elapsed time
  on the lock screen until it finishes.
- **Alerts** — finished and failed commands notify you, also while the app
  is closed through ntfy.
- **App lock** — Face ID or the device passcode locks the app, and can
  confirm every command before it runs.
- **Updates** — the app downloads improvements by itself; Settings → About
  shows the version and offers *Restart now* when one is ready.
