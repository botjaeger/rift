# Changelog — Rift for iPhone and Android

User-facing changes of the phone app, newest first. Each section is a native
build (`## <version> — <YYYY-MM-DD>`) or an over-the-air update to one
(`## <version> · update <N> — <YYYY-MM-DD>`); an update's summary is the
message its download carries. Items use `### New`, `### Improved` or
`### Fixed` sections of `- **Title** — text`.

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
