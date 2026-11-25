---
title: "Phone companion"
description: "Pair an iPhone or Android phone with Rift: follow panes live, answer coding agents, run commands and send keys, directly on your network or through an encrypted relay."
order: 5
source: "written for the site from docs/getting-started.md and docs/features.md (Remote access)"
---

The Rift app follows your computer's panes live and, if you let it, runs
commands and sends keys to them. Remote access is off until you turn it on.

## Pair a phone

1. Settings → *Remote access* → turn on *Allow remote access*.
2. *Pair a phone*, then scan the QR code with the Rift app (or the phone's
   camera). The code works once, for 2 minutes; *Cancel* withdraws it. Tick
   *The new phone can only watch* first for a phone that should not run
   commands.
3. On the phone, pick a host, a tab and a pane: its blocks follow live. Type a
   command and confirm to run it at the prompt, or use the key row (^C, esc,
   tab, arrows) to talk to a running program.

`rift-terminal remote pair` prints the same QR code and link in a terminal
(`--view` pairs a phone that can only watch).

## What a phone can do

- See windows, tabs and panes; follow a pane's blocks live, page back
  through older ones, and resume after a dropped connection without a gap;
  get notified when commands finish.
- A phone that may run commands also runs a command at a pane's prompt (it
  becomes a normal block and history entry; a pane that is running something
  answers *busy*) and sends keys (Ctrl-C, Esc, Tab, arrows, …), which reach
  the program when one is running. The computer shows a toast for each one:
  "&lt;device&gt; ran &lt;cmd&gt;" or "&lt;device&gt; sent ctrl+c".
- While a pane shows a full-screen program (vim, less, htop), the phone
  shows the pane's screen with its colours, and types into the program from
  the soft keyboard or a hardware one. The computer says "&lt;device&gt; typed into
  a program", never with the text.
- A tap on a block opens its sheet: run the command again, put it in the
  command field (*Edit*), or copy the command or its output. A running
  block's sheet offers *Stop* (ctrl+c).
- *New tab* and *New pane* start a shell, or Claude Code, Codex or OpenCode,
  on the computer. View-only phones get neither button.
- On macOS, with Rift closed, a phone that may run commands can still open a
  tab (after Face ID or the passcode on the phone). The next Rift window
  shows these shells as tabs.

## Answer a coding agent

The computer tells the phone when a pane's running command waits for you: a
password prompt (`sudo`, `ssh`), or a Claude Code or Codex question. The
phone alerts when a question starts (Yes / No / Open on the lock screen) and
docks the question at the bottom of the pane, with the file and its changed
lines. A phone that may run commands answers it; "No" can carry what to do
instead, and a password is typed, never shown, logged or kept on either side.
Each answer is ordinary input: the computer shows its toast and the pane goes
on.

## Portals: the home screen

*Portals* lists each paired computer in a card. Its line says where it is
(*This network · 30 tabs*, *Over the relay*, *Can't reach it · last seen 2 h
ago*), and under the title one line counts the tabs in words (*● 1 needs you
· 2 working · 1 failed · 27 idle*). A card shows up to five tabs; past five,
the five that matter most, and *Show all N tabs* opens the rest. The gear
opens Settings and **+** pairs another computer.

## How the phone connects

On the same network the phone connects directly. Away from it, it connects
through Rift's relay, with nothing to set up.

- **Your own relay.** Put its `wss://` address under *Connection settings* ›
  *Relay address* (host one with `rift-terminal relay`) and pair again so the
  link includes it.
- **Your network only.** Clear the *Relay address* field.
- **Cloudflare tunnel.** *Use a Cloudflare tunnel instead* replaces the relay
  with a free tunnel from this computer. Settings offers to install
  `cloudflared`; phones pair again after each restart.
- **Private network.** A Tailscale or other private-network address of the
  computer works without a relay.

The app tries the computer's own addresses first and the relay second, and
reconnects at once when the phone changes networks.

## Security

- Traffic is end-to-end encrypted (Noise XX) and the phone pins the
  computer's key.
- A relay never sees your terminal: it only forwards encrypted traffic.
- In the app, Settings › This phone › *App lock* can require Face ID or a
  fingerprint on launch or after 5 minutes away. With *Ask before running*
  on, Settings › Running commands › *Confirm with Face ID* asks before each
  command runs.

## Manage paired phones

- While a phone is connected, the tab bar shows a *Remote* chip; a click
  opens the Remote access page.
- *Paired phones* lists each phone with how it is connected, a *Can run
  commands* / *View only* choice, and *Revoke*, which disconnects it within
  a second. It cannot reconnect without pairing again.
- *Recent connections* lists the newest connections and failed attempts.
- Turning remote access off disconnects every phone.

From a terminal: `rift-terminal remote devices` lists devices,
`remote revoke ID` revokes one, `remote permission ID operate|view` changes
one, and `remote history` lists connections.

## Notifications

- *Commands I ran from here* (on by default) tells you about every command
  run from the phone.
- *Other commands* chooses *Nothing*, *Failures* or *Everything*, and *Only
  if it ran longer than* sets how long a command must run (10 s – 5 min).
- With the app closed, alerts arrive as push notifications, or through
  *Alerts through ntfy*: a random ntfy topic that the free ntfy app shows,
  with no push setup.

## Get the app

<div class="store-row">
<a class="btn btn-ghost placeholder" href="#get-the-app">[APP STORE LINK]</a>
<a class="btn btn-ghost placeholder" href="#get-the-app">[GOOGLE PLAY LINK]</a>
</div>
