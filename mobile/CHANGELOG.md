# Changelog — Rift for iPhone and Android

User-facing changes of the phone app, newest first. Each section is a native
build (`## <version> — <YYYY-MM-DD>`) or an over-the-air update to one
(`## <version> · update <N> — <YYYY-MM-DD>`); an update's summary is the
message its download carries. Items use `### New`, `### Improved` or
`### Fixed` sections of `- **Title** — text`.

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
