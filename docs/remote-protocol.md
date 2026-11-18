# Remote protocol (version 2)

The wire format between the Rift remote gateway (crate `rift-remote`, run by
`rift-terminal remote`) and the mobile companion (`mobile/`). Phase 1 of the
[mobile plan](mobile-remote-plan.md): paired devices, end-to-end encryption,
command entry and control keys on the local network; Phase 2 adds the relay
and push, Phase 3 the terminal screen for full-screen programs, typed text,
more keys and output paging (§ Phase 3). The
[threat model](mobile-threat-model.md) says what it protects.

The Rust types in `crates/rift-remote/src/protocol.rs` and the TypeScript
types in `mobile/src/protocol/types.ts` both decode every file in
`crates/rift-remote/fixtures/`; a change to the format changes those files
first, and both test suites fail until both sides follow. The Noise
interoperability transcript in `fixtures/noise/` is produced by the Rust side
(`snow`) and replayed by the TypeScript side.

## Transport

- One WebSocket per connection, **binary** frames only (a text frame closes
  the connection).
- Every connection runs the Noise handshake
  `Noise_XX_25519_ChaChaPoly_BLAKE2s` with the prologue `rift-remote/2`
  (ASCII). Handshake message 1 (client → gateway), 2 (gateway → client) and
  3 (client → gateway) are one WebSocket frame each. After it, every frame is
  one Noise transport message.
- **Pinning.** The client knows the gateway's static public key from pairing
  and aborts the handshake when message 2 carries another key.
- **Payloads.** Message 2 carries the gateway's `hello` (below); message 3
  carries the client's `hello`.
- **Messages.** After the handshake, each protocol message is a UTF-8 JSON
  object, encrypted in chunks: every Noise plaintext starts with one flag
  byte, `0x00` for the last chunk of a message and `0x01` when more follow,
  then at most 65 000 bytes of the JSON text. A message is decoded once its
  last chunk arrives.
- Every JSON object carries `"t"`, its type. Unknown `"t"` from the client
  gets `error` `bad_request`; the client ignores unknown types (forward
  compatibility).
- The gateway listens on `0.0.0.0:7878` by default (`remote_listen`); the
  pairing link lists every non-loopback IPv4 address it is reachable on.

### Handshake payloads

Gateway (message 2):

```json
{"v":2,"host":"studio-mac","os":"macos","version":"0.1.0"}
```

`host` (here, in `welcome` and in the pairing link's `n`) is the computer's
name as its owner sees it: macOS's ComputerName, Linux's `PRETTY_HOSTNAME`
from `/etc/machine-info`, else the network hostname. The app shows the
latest one it received.

Client (message 3), from a paired device:

```json
{"v":2,"name":"Sam's iPhone","platform":"ios"}
```

and when pairing (first connection with a pairing secret):

```json
{"v":2,"name":"Sam's iPhone","platform":"ios","pair":"<pairing secret, base64url>"}
```

- The gateway accepts the connection when the client's static key is a
  paired device, or when `pair` is a live pairing secret: it then records the
  device (name, platform, static key, the secret's permission) and spends the
  secret. Name and platform are kept to 64 characters without control or
  bidirectional formatting characters (the desktop's toasts drop those from
  a command too). Anything else gets `error` `auth` and the socket closes.
- `v` other than 2 gets `error` `version` and the socket closes.
- The first message after the handshake is the gateway's `welcome`.

## Pairing link

`rift-terminal remote pair` and Settings › Remote access › *Pair a phone*
create a one-time secret (16 random bytes, valid 2 minutes, one use) and
show it as a QR code and as text. Settings' *Cancel* (and changing *The new
phone can only watch* while a code shows) removes the secret from
`pairing.json` at once:

```
rift-remote://pair?a=192.168.1.29:7878,10.0.0.5:7878&k=<gateway static public key, base64url>&s=<secret, base64url>&n=studio-mac&p=operate
```

- `a`: addresses to try in order; `k`: the key the client pins; `s`: the
  secret; `n`: the host name to show; `p`: `operate` or `view`.
- The app opens this link from its scanner, from a paste, or as a deep link
  (the system camera scanning the QR opens it).

## Devices and permissions

- `operate`: everything below. `view`: no `run`, `keys`, `text`, `paste`, `mouse`, `fit`, `open`, `close`, `move`, `start`,
  `upload` or
  `complete` (its history can hold secrets typed on a command line, and
  path completion lists any folder), and no `diff` of an untracked file
  (its whole content: a `.env`, a key); they get `ack` with `error`
  `forbidden`. `changes` still lists untracked files to `view` devices
  (path, status and line counts, no content). A `view` device may `scroll`
  a pane's own view (the primary screen), not send its program the wheel
  (wheel reports, or arrow keys under alternate scroll): that `scroll` is
  `forbidden`.
- The desktop lists, renames nothing, and revokes devices (Settings ›
  Remote access, or `rift-terminal remote devices` / `revoke ID`). A revoked
  device's open connections close within 1 second, and its key is refused
  from then on.
- A client may remove itself with `forget`.

## Limits

| Limit | Value |
|---|---|
| Client message | 4 KiB of JSON (an `upload` chunk 62 KiB); larger closes the connection. The larger limit holds only for an authenticated `operate` device and only for a message whose `t` is `upload` (told before the rest is parsed); handshake frames and a `view` device's frames stay within the 4 KiB message's frame |
| Gateway message | 1 MiB of JSON (in ≤ 65 000-byte chunks); an answer to a request with an `id` that would be larger (JSON escapes can grow a bounded page several times) is an `ack` `too_large` instead |
| Concurrent connections | 8; the ninth is refused with `error` `busy` after the handshake, unless its key has connections silent for half the idle timeout: those end instead (what a dropped network left behind) |
| Handshake deadline | 5 s after the socket opens |
| Failed handshakes | 10 per minute per remote address; then refused for a minute |
| Idle | the gateway pings every 15 s and drops a client silent for 45 s |
| Blocks per `snapshot` / `older` | 30, newest last |
| A block's `command` | at most 16 KiB (cut on a character boundary) |
| Output per block | the last 500 lines and at most 64 KiB; `dropped` counts the lines before them (a row the scrollback limit already discarded counts as one line) |
| Replay ring | the last 256 `update`s per pane |
| Poll interval | 100 ms for subscribed panes (a pane's screen every 16 ms for 600 ms after a `scroll`, `keys` or `text` to it), 1 s for the topology |
| `run` | 4 per second per connection; the command at most 2 KiB, one line |
| `keys` | 20 keys per message, 20 messages per second per connection |
| `text` | 1 KiB per message, one line without control characters; counts against the `keys` rate |
| `paste` | 48 KiB of text per message, line breaks and tabs the only control characters; counts against the `keys` rate; only an `operate` device's message may pass 4 KiB |
| `mouse` | a click per message; counts against the `keys` rate |
| `open`, `close`, `start` | 2 per second per connection, together |
| `fit` | 4 per second per connection; `cols` 20 to 300, `rows` 5 to 120 (clamped) |
| `scroll` | at most 50 lines either way (clamped), 40 per second per connection (apart from `keys`); every line of a `scroll` reaches the program as a wheel report or an arrow key |
| `history` | at most 200 lines and 1 MiB per answer (a longer range keeps its end; lines leave from its start until it fits), 10 requests per second per connection |
| `upload` | an image of at most 10 MiB, PNG or JPEG by its first bytes, or with `ext` (1 to 8 of `a-z0-9`) a file of any bytes; chunks of 1 to 45 000 bytes (60 000 base64 characters); one upload in flight and 1 started per second per connection; at most 2 in flight across the gateway; 60 s from the first chunk to the last |
| `output` | 1 to 500 lines and at most 64 KiB per answer, 10 requests per second per connection |
| `screen` | at most 300 columns and 120 rows (the top-left part of a larger screen), one per 100 ms (per 16 ms for 600 ms after a `scroll`, `keys` or `text`) |
| `complete` | the line at most 1 KiB; at most 50 `items` and 50 `history` entries of 1 KiB each; shares `output`'s rate |
| A pane's `waiting` / `running` | prompt 300 characters, 9 choices of 200, 12 context lines of 200; `running.line` 200 characters |
| `topology` text | a tab's `title`, a pane's `job` and `last_command` at most 1024 characters each (cut) |

## Client → gateway

```json
{"t":"subscribe","pane":12,"after":null}
{"t":"subscribe","pane":12,"after":41}
{"t":"older","pane":12,"before":87}
{"t":"unsubscribe"}
{"t":"run","id":"r1","pane":12,"command":"cargo fmt --all"}
{"t":"keys","id":"k1","pane":12,"keys":["ctrl+c"]}
{"t":"forget"}
{"t":"ping","n":3}
{"t":"push","token":"ExponentPushToken[…]","ntfy":null,"long_ms":30000,"show_command":false,"failures_only":true}
{"t":"push","token":null,"ntfy":"https://ntfy.sh/rift-x7Kq2mP9vT4wL8nR3sY6hZ","long_ms":30000,"show_command":false,"failures_only":false}
{"t":"push","token":null,"ntfy":null,"long_ms":null,"show_command":false,"failures_only":false}
{"t":"notify","pane":12}
{"t":"notify","pane":12,"off":true}
{"t":"visible","on":false}
{"t":"text","id":"x1","pane":12,"text":":wq"}
{"t":"paste","id":"p1","pane":12,"text":"first line\nsecond line\n\tindented"}
{"t":"mouse","id":"m1","pane":12,"col":4,"row":7}
{"t":"screen","pane":12,"mode":"auto"}
{"t":"output","id":"o1","pane":12,"block":86,"from":0,"limit":500}
{"t":"open","id":"n1","pane":12,"kind":"tab"}
{"t":"close","id":"c1","pane":12,"kind":"pane"}
{"t":"close","id":"c2","pane":12,"kind":"tab"}
{"t":"move","id":"v1","pane":12,"kind":"tab","to":0}
{"t":"move","id":"v2","pane":14,"kind":"pane","to":1}
{"t":"chip","id":"m5","pane":12,"kind":"docker"}
{"t":"reset_elapsed","id":"r7","pane":12}
{"t":"scroll","id":"s1","pane":12,"lines":3}
{"t":"upload","id":"u1","pane":12,"size":68,"offset":0,"data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="}
{"t":"upload","id":"u2","pane":12,"size":5,"offset":0,"data":"aGVsbG8=","ext":"txt"}
```

- `subscribe` follows one pane (a new `subscribe` replaces the old one).
  `after` is the last `seq` the client applied for that pane: when the ring
  still holds every `update` after it, the gateway replays them; otherwise (or
  with `after: null`) it sends a fresh `snapshot`.
- `older` asks for up to 30 blocks older than block id `before`.
- `run` submits a command line to a pane that is at its prompt, exactly as if
  it were typed in the pane's editor and Enter pressed: it becomes a block,
  goes into history, and the desktop shows a toast naming the device. A pane
  that is running something answers `ack` with `error` `busy`; use `keys`
  there. The command may not contain a newline or a control character.
- `keys` sends keys to the pane as if pressed on the desktop (raw to the
  program when one is running, to the editor at a prompt). At a prompt no
  key puts text on the command line: `shift+enter`, `alt+enter`,
  `alt+space`, `up`/`down` (history), `ctrl+r` (history search), `tab`
  (completion) and `ctrl+right` (the ghost suggestion) do nothing, and
  `enter` while the command line has text (the desktop user's draft)
  answers `ack` with `error` `busy`, message `the command line has text;
  send the command with run` (an empty line's `enter` passes). A command
  line only arrives through `run`. A name is
  optional modifiers, in this order and each at most once, `ctrl+`,
  `alt+`, `shift+`, then a base key: `esc`, `tab`, `enter`, `backspace`,
  `delete`, `insert`, `up`, `down`, `left`, `right`, `home`, `end`,
  `pageup`, `pagedown`, `f1` … `f12` (with any modifiers or none); `a` …
  `z` and `space` only with `ctrl+` and/or `alt+` and without `shift+`
  (plain letters are `text`). So `ctrl+c`, `shift+tab`, `alt+b`,
  `ctrl+alt+delete`, `shift+f5`, `ctrl+space`; not `ctrl+A`, `shift+ctrl+c`,
  `shift+a`, `a`, `space`.
- `text` types printable text into the program running in the pane, as if
  typed on the desktop's keyboard (the program's keyboard mode applies;
  under the kitty keyboard protocol's "report all keys" each character is
  one key press). It is one line without control characters (`enter`,
  `tab`, `backspace` are `keys`), at most 1 KiB. A pane at its prompt, or
  one whose submitted command the shell has not started yet, answers `ack`
  with `error` `busy`: a command line goes through `run`, so it is always
  confirmed as one. Two exceptions, where the gateway cannot tell a
  prompt from a program, take text: a shell without Rift's shell
  integration, and a prompt drawn on the alternate screen (tmux, screen,
  an ssh session running one). A pane whose last block reads as a prompt
  while a program other than the shell holds its terminal (a reattached
  agent whose replay lost its command's start, a program printing a prompt
  marker) is running that program: it takes text and images, as the
  topology's `running` says, and `run` is `busy` there. The desktop's
  toast names the device, never the text (it may be a password).
- `screen` sets what the gateway sends of the subscribed pane's terminal
  screen (§ Phase 3): `off` (the default after every `subscribe`), `auto`
  (while the pane shows the alternate screen: vim, less, htop) or `on`
  (always). `pane` must be the subscribed pane, else `error` `bad_request`;
  a `screen` for the pane whose `subscribe` was just refused (`no_pane`: it
  closed while the app was away) is dropped without a second error.
- `output` asks for lines `from` … `from + limit - 1` of a block's output
  (§ Phase 3).
- `fit` (`{"t":"fit","id":"f1","pane":12,"cols":64,"rows":40}`, feature `fit`)
  sets the phone's size for the subscribed pane, so programs reflow to its
  width (§ Phone-sized panes); `{"t":"fit","id":"f2","pane":12,"cols":null,"rows":null}`
  releases it. `cols` and `rows` are both numbers or both `null`, else `ack`
  with `error` `bad_request`; so is a `fit` for a pane the client does not
  follow. Answered with `ack` like `keys`; `view` devices get `forbidden`.
- `open` (feature `open`) opens a shell next to `pane`: `kind` `tab` is a
  new tab in the pane's window, `pane` a new pane split to its right; both
  start where a new pane from that pane would (its folder, or the
  desktop's `start_dir`). The desktop shows it and a toast naming the
  device. Every connection gets the `topology` holding the new pane before
  the `ack`, which names it in `pane`. A window that cannot split the pane
  answers `ack` with `error` `busy`.
- `close` (feature `close`) ends a shell: `kind` `pane` closes `pane` (its
  shell ends, as ⌘W on the desktop), `tab` closes the whole tab holding
  it. The desktop does not ask about running commands (the phone asks its
  user first) and shows a toast naming the device; closing the window's
  last tab closes the window, as ⌘W does. Every connection gets the
  `topology` without the closed panes before the plain `ack` (`{"t":"ack",
  "id":"c1","ok":true}`). A pane that does not exist answers `ack` with
  `error` `no_pane`; `view` devices and a desktop with `remote_access` off
  answer `forbidden`. A background shell (see `start`) is a tab of one
  pane: both kinds end it.
- `chip` (feature `prompt_chips`) asks what a pull request, interpreter, Docker
  or Time on tab chip opens: `kind` is `pr`, `python`, `php`, `docker` or
  `elapsed` (§ Prompt chip menus); answered with `chip`.
- `move` (feature `move`; `{"t":"move","id":"v1","pane":12,"kind":"tab","to":0}`) reorders: `kind`
  `tab` moves the tab holding `pane` to position `to`, counted over the tabs of the pane's
  desktop process as `topology` lists them (a tab's `index`); `kind` `pane` moves `pane` to
  position `to` among its tab's panes (0-based, as listed), exchanging its place in the split
  layout with each pane it passes. The desktop keeps the active tab and pane, and refuses
  (`ack` `busy`) a position outside the pane's own window or tab, or across the pinned / group
  zone of a tab (as its own tab drag). Every connection gets the new `topology` before the plain
  `ack`; `no_pane` for an unknown pane; `view` devices and a desktop with `remote_access` off answer
  `forbidden`. A background shell cannot be moved. Limit: 4 per second per connection, apart from
  `open` / `close`.
- `reset_elapsed` (feature `prompt_chips`) restarts the clock of the tab
  holding `pane`, as the Time on tab card's Reset does on the desktop. Answered
  with `ack`; `view` devices and a desktop with `remote_access` off answer
  `forbidden`, an unknown pane `no_pane`. Every connection gets the new
  `topology` before the `ack`.
- `start` (`{"t":"start","id":"n2"}`, feature `start`) starts a shell on a
  computer whose user allows it while no Rift window is open
  (macOS, `remote_access` with durable sessions): the gateway keeps the shell in the session
  service and lists it in a background window (`{"pid":0,"background":true,
  "tabs":[…]}`; at most 8 shells), whose panes take `subscribe`, `run`,
  `keys`, `text`, `screen`, `output` and `open` `tab` (another background
  shell) like any other. With a window open it opens a tab there instead.
  Every connection gets the `topology` with the new pane before the `ack`,
  and the computer shows a notification naming the device. When Rift opens
  it shows the background shells as tabs, and the background window leaves
  the `topology`. A computer that does not allow it answers `ack` with
  `error` `forbidden`.
- `scroll` (feature `scroll`) scrolls `pane` as the desktop's mouse wheel
  over it would: `lines` up (negative: down, toward the newest), at most 50
  either way (clamped; 0 does nothing). The desktop decides what that is
  from the pane's terminal modes, exactly as for its own wheel: a program
  that tracks the mouse (Claude Code's fullscreen renderer, vim with
  `mouse=a`, htop) gets that many wheel reports at the middle of
  its screen, with no modifiers; a full-screen program without mouse
  tracking under alternate scroll (DECSET 1007: less, man) gets arrow keys
  in its cursor mode; any other alternate screen nothing; on the primary
  screen the phones' own view of the pane moves: the styled screen that
  `screen` `on` sends, scrolled within that screen's own list of lines
  (clamped to its top, so scrolling back down starts at once). The
  desktop's view never moves for a phone, and the desktop's scrolling never
  moves the phones'. The phones' view stays on what it shows as new output
  arrives (as the desktop's does), and follows the end again once it is
  scrolled back down or a phone sends `run`, `keys` or `text` to the pane.
  Answered with `ack`. A `view` device may move the view; where the wheel
  would reach the program it gets `forbidden`. A pane in the background
  window (`start`) has no view: `busy`. No toast: scrolling is not typing.
- `paste` (feature `paste`) gives text to the program running in `pane` as one
  paste: bracketed where the program asked for bracketed paste (DECSET 2004),
  so its line breaks submit nothing. Several lines to a program without it are
  refused (`the program takes one line at a time`), never typed: each break
  would press Enter. Any other control character is `bad_request`, so no
  escape sequence leaves the paste; over 48 KiB is `too_large`. A pane at its
  prompt is `busy`, as for `text`.
- `mouse` (feature `mouse`) is a left click, press and release, at a 0-based
  cell of `pane`'s screen, for a program that tracks the mouse. Any other
  program answers `the program does not track the mouse`, and the phone says
  nothing: a tap there simply does nothing.
- `upload` (feature `upload`) pastes an image into the program running in
  `pane`, as a file dropped on the desktop window is: the desktop saves it
  and pastes its path (shell-quoted, a space after), which Claude Code and
  Codex read as an attached image. The image goes in chunks: `size` is its
  length in bytes (1 to 10 MiB), every chunk carries the same `id`, `pane`
  and `size`, `offset` is the chunk's first byte (0, then the bytes sent
  so far: in order, no gaps) and `data` 1 to 45 000 bytes in standard
  base64 (with padding; a chunk of whole 3-byte groups decodes on its own).
  A chunk at offset 0 starts a new upload and drops one still arriving.
  The gateway answers once: `ack` after the last chunk once the path is
  pasted, or an error `ack` at the first chunk that fails, whose later
  chunks it then drops unanswered. The first chunk is checked against the
  pane before any more arrive (a pane at its prompt is `busy` at once).
  The gateway holds `size` bytes for the image from its first chunk, at
  most 2 images across all connections at once, and drops one that has not
  arrived whole within 60 s of its first chunk. The type comes from the
  first bytes only, PNG (`89 50 4E 47 0D 0A 1A 0A`) or JPEG (`FF D8 FF`),
  else `bad_request`; nothing the client sends names the file. Errors, by
  `message`:
  - `forbidden`: `this device may only view` (`view` devices),
    `remote access is off on the desktop`.
  - `rate`: more than one upload started a second.
  - `too_large`: `size` over 10 MiB.
  - `bad_request`: `the image is empty`, `not a PNG or JPEG image`, `an
    upload chunk out of order` (a gap, another `id`, `pane` or `size`),
    `data is not base64`, `a chunk holds 1 to 45000 bytes`, `more bytes than
    the upload's size`.
  - `busy`: `pane N is at its prompt; an image goes to a running program`
    (also while a submitted command has not started yet, as for `text`),
    `the image took longer than 60 s to arrive; try again` (past the
    deadline, answered without waiting for another chunk), `other images are
    arriving; try again in a moment` (2 already), `the desktop cannot hold
    the image now` (its memory refused `size`), `images and files go to a shell in a
    Rift window` (a background shell), `cannot save the upload: …` (the
    uploads folder is not writable, or is not a folder: a link there is
    refused), or the window's own reason when it did not answer.
  - `no_pane`: `no pane N`.

  The desktop shows a toast naming the device ("<device> pasted an
  image").

  With `ext` (feature `upload_files`) the upload is a file of any bytes,
  never sniffed: `ext` is its extension, 1 to 8 of `a-z0-9`, and every chunk
  carries the same one. All of the above holds (operate only, a running
  program, 10 MiB, the rates and the deadline), with "file" for "image" in
  each message, plus `bad_request` `ext holds 1 to 8 of a-z and 0-9` (`PNG`,
  `../x`, nine characters, empty). The phone's file name never leaves it:
  the gateway makes the name and only `ext` ends it. The file is saved 0600,
  nothing runs it, and only its shell-quoted path is pasted, with no Return.
  Without `ext` a GIF or anything else that is not a PNG or JPEG stays refused;
  a gateway without the feature refuses a file that way too, so the app checks
  the feature first. The toast says "<device> pasted a file" for anything but
  `.png` and `.jpg`.
- `id` is chosen by the client (at most 64 characters) and returned in `ack`
  (or, for `output`, in the `output` answer).
- `forget` removes this device from the desktop's paired devices and closes
  the connection.
- `ping` is answered with `pong` carrying the same `n`.
- `push`, `notify` and `visible`: alerts while the app is closed (see § Push). No
  reply; a malformed one gets `error` `bad_request`.

## Gateway → client

```json
{"t":"welcome","v":2,"host":"studio-mac","os":"macos","version":"0.1.0","device":"d3f1","permission":"operate","features":["screen","text","output","push_failures","push_ntfy","git","notify_off","chips","waiting","complete","running","open","close","move","fit","prompt_chips","closing","scroll","upload","upload_files","paste","mouse","clipboard","history","stitch"],"addresses":["192.168.1.29:7878"]}
{"t":"topology","windows":[{"pid":4242,"tabs":[{"index":0,"title":"rift","active":true,"panes":[{"id":12,"active":true,"cwd":"/Users/me/rift","job":null,"last_command":"cargo test","last_exit":0,"git":{"branch":"main","ahead":1,"behind":0,"dirty":true,"files":3,"added":42,"removed":7}}]}]}]}
{"t":"snapshot","pane":12,"seq":41,"blocks":[Block],"has_older":true}
{"t":"update","pane":12,"seq":42,"blocks":[Block],"removed":[80]}
{"t":"older","pane":12,"blocks":[Block],"has_older":false}
{"t":"ack","id":"r1","ok":true}
{"t":"ack","id":"r2","ok":false,"error":"busy","message":"pane 12 is running a command"}
{"t":"ack","id":"n1","ok":true,"pane":14}
{"t":"done","pane":12,"block":87,"command":"cargo test","exit":0,"duration_ms":260500}
{"t":"pong","n":3}
{"t":"theme","theme":Theme}
{"t":"closing","reason":"update","back_in_s":60}
{"t":"closing","reason":"quit"}
{"t":"error","code":"auth","message":"this device is not paired"}
{"t":"screen","pane":12,"cols":80,"rows":24,"alt":true,"cursor":[23,1],"full":false,"lines":[[0,[["top - 10:02:11 up 3 days",null,null,1]]],[7,[]]]}
{"t":"screen","pane":12,"cols":80,"rows":24,"alt":false,"cursor":null,"full":true,"fg":"d4d4d4","bg":"1e1e1e","rules":[[0,"3fb950"],[1,"3fb950"]],"lines":[[0,[["▾ ls",null,null,1],["✓ 0","3fb950","1a3a22",0]]],[1,[["a.txt",null,null,0]]]]}
{"t":"screen_end","pane":12}
{"t":"output","id":"o1","pane":12,"block":86,"from":0,"text":"   Compiling rift-pty v0.1.0\n","total":40}
```

- `welcome` carries `addresses`: where the phone can reach the gateway
  now, `host:port`, at most 16 (the configured ones, else the computer's
  current interface addresses; absent from older gateways and when there is
  no network). See Relay for how the app uses them.
- `welcome` follows the handshake, then `topology`, which is sent again
  whenever it changes. A pane's `id` is the gateway's: the window's pid
  times 2^24 plus the window's own pane id (every window numbers its panes
  from 1), so it names one pane among all windows. Tabs come in the
  window's tab-strip order, a tab's panes in its layout order (left to
  right, top to bottom), not the order they were opened in. A tab pinned on
  the desktop's tab bar carries `"pinned":true` (absent when not); pinned
  tabs come first, as on the desktop. `features` lists what this gateway serves beyond
  Phase 2: `screen`, `text`, `output` (§ Phase 3), `push_failures`
  (`push.failures_only`, § Push), `push_ntfy` (`push.ntfy` and `visible`, § Push), `git`
  (a pane's `git` in `topology`, below), `notify_off` (`notify` with
  `off`, § Push), `chips` (`dirs`, `branches`, `changes`, `diff`, §
  Phase 3), `waiting` (a pane's `waiting` in `topology`, below),
  `complete` (§ Phase 3), `running` (a pane's `running` in
  `topology`, below), `fit` (§ Phase 3), `prompt_chips` (a pane's `chips`
  in `topology`, below, and the `chip` and `reset_elapsed` requests, §
  Prompt chip menus), `open` and `close`, `move` (tab and pane reorder), `closing` (below), `scroll` and
  `upload`, `upload_files`, `paste` and `mouse` (above), `clipboard`, `theme` and `history` (below), and `start` (listed only while
  the computer allows it: macOS, `remote_access`, durable sessions). A
  gateway that leaves
  the field out serves none of them; the app then hides what needs them.
- A pane's `git` (gateways with the `git` feature) is the git status the
  desktop shows in its own prompt chips for that pane: `branch` (a short
  commit id when detached), `ahead` and `behind` its upstream, `dirty`
  (any change, untracked files included), and the uncommitted diff against
  HEAD as `files` changed, lines `added` and `removed`. It is absent when
  the pane is not in a repository or the desktop has not looked yet. The
  desktop refreshes it after a `cd` or a finished command, and every 3 s
  while a window is in use or a phone is connected: the panes of each
  window's active tab, and with a phone connected the panes of every tab.
  Without a phone, a pane in another tab keeps the value from when its tab was
  last active. Without the feature the field never appears
  and the app shows no git chips.
- A pane's `chips` (gateways with the `prompt_chips` feature) are the prompt
  chips the desktop draws above that pane's command line, exactly as it
  renders them: the pane's configured `prompt_chips` order, only those with
  something to show right now (none in shell-prompt mode), computed by the
  desktop's own chip code (`crates/rift-terminal/src/views/prompt_chips.rs`).
  Each is `{"kind":"docker","icon":"accent","full":[{"text":"flow","tone":"bright"},
  {"text":"3/4","tone":"success"}],"short":[{"text":"3/4","tone":"success"}],"attention":true}`.
  `kind` is the chip (`cwd`, `git_branch`, `git_diff`, `user`, `host`,
  `python`, `php`, `github_pr`, `docker`, `date`, `time12`, `time24`,
  `elapsed`); `full` is what the desktop shows, `short` the same shortened
  for a narrow row (a folder's name, the branch with its dirty dot, `102 +2.1k
  −970`, `#42`, a version without its patch, `3/4`) and equal to `full` for
  the chips with nothing to cut; both are runs of `{text, tone}`. `tone` names
  the desktop theme's colour role, so the phone draws its own palette:
  `bright`, `ui`, `dim`, `muted` (`text_bright`, `text_ui`, `text_dim`,
  `text_muted`), `success`, `error`, `warning`, `accent`, `magenta` (ANSI
  magenta, a merged pull request) and `indigo` (PHP's mark); `icon` is the
  glyph's tone. `attention` (left out when false) means the chip needs a look:
  a Docker container down or unhealthy. At most 16 chips of at most 6 runs of
  at most 200 characters. An app ignores a `kind` or `tone` it does not know
  (it draws the text in `ui`). The field is absent from a desktop without the
  feature; the app then draws a folder chip and, with `git`, the branch and
  changed files.
- A pane's `waiting` (gateways with the `waiting` feature) says the pane's
  running command waits for the user, and how to answer it:
  `{"kind":"choice","prompt":"Do you want to make this edit to session.rs?","choices":["Yes","Yes, allow all edits during this session (shift+tab)","No, and tell Claude what to do differently (esc)"],"selected":0,"numbered":true,"context":["Edit file","…"]}`.
  `kind` is `password` (the terminal reads a line without echo in line
  mode — `sudo`, `ssh`, `read -s` — or the cursor line reads like a
  password prompt; any program), `choice` (a coding agent's menu with its
  `❯` / `›` selector on the screen's last rows, Claude Code and Codex
  style; only while the pane runs a coding agent, so another program's
  menu-shaped output never takes a phone's answer) or `input` (an unread
  "needs your input" for the pane in the desktop's attention center, e.g.
  an agent's OSC 9 / 777 notification, while its command runs and until
  anything is typed into the pane — focus and mouse reports the terminal
  sends do not count; the phone answers with text its user types).
  `prompt` is the
  question (at most 300 characters), `choices` the menu's entries without
  their numbers (at most 9, each at most 200 characters), `selected` the
  entry under the selector, `numbered` whether the entries are numbered,
  `context` up to 12 screen lines above the question (each at most 200
  characters). `password` and `input` carry no choices and no context.
  Absent when nothing waits. A phone answers any choice, `numbered` or not, with `up` / `down`
  from `selected`, then `enter` (`keys`). A password is answered with
  `text` and `enter`, never echoed or kept.
- A pane's `running` (gateways with the `running` feature) describes its
  running command for a list of everything running without following each
  pane: `{"started":1760364000,"line":"test session::resize ... ok"}`.
  `started` is when the command started (Unix seconds, the desktop's
  clock; null without shell integration), `line` the screen's last
  non-empty line at or above the cursor (at most 200 characters; empty
  while a full-screen program runs). Absent at the prompt. A coding agent
  (Claude Code, Codex) adds `title`, its name for the session (the pane's
  own title without the agent's status mark, at most 200 characters), and
  `"idle":true` while it waits at its own prompt (it runs, but is not
  working), and `"agent":true` (with or without a `title`: an agent whose
  title is empty has none); all three are absent for any other program,
  and an app reads a missing `idle` as working. An agent may draw on
  either screen (Claude Code picks its renderer per launch: the classic one
  on the primary screen, the fullscreen one on the alternate screen), so
  the app gives every `agent` pane its message input. The topology is sent again when `line`
  changes, at most once per topology poll.
- `seq` is per pane and per gateway run: `update` N+1 follows `update` (or
  `snapshot`) N. A client that sees a gap re-subscribes with its last `seq`.
  A run's first `seq` is the gateway's start time in milliseconds, not 0, so
  a `subscribe{after}` carrying a `seq` of an earlier run (a gateway that
  restarted) never lines up with the new run's replay ring and is answered
  with a `snapshot`; a client treats `seq` as an opaque counter.
- `update.blocks` are whole blocks that are new or changed since the
  previous `seq` (upsert by `id`); `removed` lists ids gone from the pane
  (`clear`, scrollback trim), including ids the client received through
  `older`. A running block's clock alone is not a change: `duration_ms` is
  current whenever the block is sent for another reason.
- No gateway message exceeds 1 MiB: a larger `update` is split into several,
  each with its own `seq`; a `snapshot` or `older` page that would not fit
  leaves out its oldest blocks and reports `has_older: true`.
- `done` is sent to every connection when any pane's command finishes (for
  notifications), whether or not the client follows that pane.
- A pane that closes is reported by `topology` and by `error` `no_pane` for
  its subscription.
- `closing` (gateways with the `closing` feature) is sent to every
  connected, authenticated device just before the gateway stops, after
  what was queued for it before (also to a full queue), and to a device
  that connects after it right after its `topology`. `reason` is `quit`
  (Rift quit on the computer; the gateway is gone for good) or `update`
  (Rift is updating and restarting; nothing is wrong); `back_in_s` is about
  when the gateway is back, in seconds (the desktop sends 60 with
  `update`), left out when unknown. The desktop calls
  `Gateway::announce_closing`, then `Gateway::flush` to wait briefly until
  every connection has written it, then stops. An app that does not know
  the type ignores it (every app ignores an unknown `t`), so it needs no
  negotiation. The app remembers it until the next `welcome`, and when the
  connection then drops:
  - `update`: it reconnects every 3 s for up to 180 s after the message,
    without counting those rounds against its 5 retries; the computer's
    card says "Updating Rift · back in about a minute" (the time left of
    `back_in_s`, then "taking a little longer than expected") and the pane
    "*host* is updating Rift", with no retry counter. After the 180 s it
    falls back to its usual backoff and its 5 retries, counted from the
    first. Cancel stops it like any reconnect; Try again tries at once.
  - `quit`: it does not retry; it stops with "Rift quit on this computer"
    until Try again, the phone coming back online, or the app opening.
- Error codes: `auth`, `version`, `bad_request`, `busy`, `forbidden`,
  `no_pane`, `no_block`, `too_large`, `rate`. `ack.error` uses the same
  codes.

### Block

```json
{
  "id": 87,
  "command": "cargo test --workspace",
  "state": "running",
  "exit": null,
  "duration_ms": 252000,
  "cwd": "/Users/me/rift",
  "branch": "main",
  "output": "test block::tests::osc133_idempotent ... ok\n",
  "dropped": 0,
  "restored": false
}
```

- `state`: `running` or `done` (a prompt being typed is not sent).
- `exit`: the exit status once `done` (may stay `null` when the shell did
  not report one).
- `duration_ms`: elapsed time while `running`, final duration when `done`.
- `output`: plain text, no escape sequences; soft-wrapped rows are joined.
- `dropped`: the number of output lines before `output`, which is also the
  line number of its first line (`output` request, § Phase 3). A row
  already gone from the window's scrollback counts as one line.
- `restored`: replayed from a saved transcript of an earlier session.

## Relay (Phase 2)

For use outside the local network, a gateway can also be reached through a
relay: `rift-terminal relay` (or the `packaging/relay.Dockerfile` image),
which anyone can host. The relay only splices WebSocket connections; it
never holds a key and only ever sees Noise ciphertext (and the routing id).
It listens on plain `ws://`; put it behind a TLS reverse proxy and give
gateways and phones its `wss://` URL. Without a server, `remote_tunnel`
("Use a Cloudflare tunnel instead") makes the gateway run the relay itself on a
loopback port behind a Cloudflare quick tunnel and register at the
tunnel's `wss://` address (`scripts/relay-local.sh` does the same with the
Docker image); the address changes on every start, so phones pair again
each time. Nothing on the wire changes: it is an ordinary relay, except
that it lets only this computer's gateway register (any other routing id is
closed after its key proof, as an unproven one is) and holds at most 192
open sockets (every one arrives through the tunnel, so none is limited per
address; the bound keeps a flood from spending the gateway's threads).

- **Routing id**: base64url of the first 16 bytes of SHA-256 over the
  gateway's static public key. It names the gateway at the relay and is no
  secret.
- **Gateway registration**: the gateway (config `remote_relay = "wss://…"`)
  keeps one control connection open to `<relay>/gateway` and sends
  `{"t":"register","id":"<routing id>","key":"<static public key, base64url>","secret":"<32 random bytes, base64url>"}`.
  The relay closes it unless `id` is `key`'s routing id, then checks that
  the gateway holds the key: it sends `{"t":"prove","m":"<base64url>"}`,
  message 1 of a `Noise_NK_25519_ChaChaPoly_BLAKE2s` handshake (prologue
  `rift-relay/1`, empty payload) to `key`, and the gateway answers
  `{"t":"proof","m":"<base64url>"}` with message 2; a proof that does not
  read closes the connection. So only the key's holder can claim its id:
  a proved registration replaces any earlier control connection and secret
  of that id (a reconnect or a restart). When the relay holds 1000 ids it
  forgets the one offline the longest (without pairs) to make room, and
  answers `{"t":"error","code":"full"}` only when every id is online or
  busy. The HTTP upgrade and the whole registration must finish within
  10 s of the socket opening. The relay pings control connections every 20 s and drops one silent for
  60 s; the gateway pings every 15 s, redials after 45 s of silence, and
  backs off 1 s → 30 s between attempts. A path in the relay URL is kept
  (`wss://host/rift` → `/rift/gateway`). The secret is kept in
  `<data dir>/remote/relay.json` (0600) so restarts keep the id. A
  successful registration is answered with `{"t":"registered"}`.
- **Phone connection**: the phone opens `<relay>/connect?id=<routing id>`.
  The relay sends `{"t":"open","conn":"<random 16-byte id, base64url>"}` on
  the gateway's control connection; the gateway opens
  `<relay>/accept?id=<routing id>&conn=<conn>` with the header
  `Authorization: Bearer <secret>` (never in the URL, which proxies log) and
  the relay then forwards every binary frame between the two sockets
  unchanged, in both directions. The gateway treats that socket like a
  direct one (same Noise handshake and limits), but relay sockets have
  their own budget of 8 sessions and 16 sockets, so `open`s, which anyone
  who knows the routing id can cause, never refuse a phone on the LAN.
  A phone whose gateway does not accept within 10 seconds is closed. Text
  frames on a spliced pair close both. Through the relay, WebSocket pongs
  come from the relay itself, so the gateway's 45 s idle limit counts only
  the phone's Noise messages (the app pings in them every 15 s).
- **Relay limits**: 32 pairs per id (waiting and spliced), 1 MiB per frame,
  60 s idle per pair (Noise pings keep live ones open), 30 `/connect` per
  minute per remote address, 64 open sockets per address, 1000 registered
  ids, 4096 sockets, 10 s for an upgrade in total (a request sent a byte at
  a time is cut there). Addresses count per IPv4 address or IPv6 /64.
  Control messages are text frames of at most 1 KiB. Refusals are HTTP
  statuses at the upgrade: `/connect` 400 (bad id), 404 (gateway offline),
  429 (rate), 503 (pairs full); `/accept` 400 (no secret), 403 (wrong secret
  or id), 404 (unknown conn). Behind a reverse proxy, `--trust-forwarded`
  takes the client address from the last `X-Forwarded-For` entry (the one
  the proxy wrote; when it is not an address, the peer's) and does not
  count sockets per address.
- **Pairing link**: `r=<relay URL>` is added when the gateway has a relay;
  when pairing, the phone tries the `a` addresses first (1.5 s each), then
  the relay. A paired host's rounds start at the path that last reached
  `welcome` and go on through the rest in order (addresses, then the relay,
  wrapping around); a network change starts at the first address. A LAN
  address that answers with another key than the pinned one is skipped; a
  mismatch on the relay, or on every path of a round, stops the client
  (`key_mismatch`) until the user retries or the network changes.
- **Addresses that change**: the computer moves between networks, so the
  link's addresses go stale. The app puts a `welcome`'s `addresses` first
  and the older ones after them, 4 addresses in all (all of the welcome's
  when it names more). Over the relay, it opens each new
  address beside the relay (1.5 s); the first that answers takes the
  connection over, its handshake on that socket, and the relay closes.
  None that answers leaves the relay as it was. `status.json` (what
  `remote pair` lists) follows the interfaces too.
- **What the relay learns**: which routing ids are online, when phones
  connect, from which addresses, and how many bytes flow. Never content,
  keys or device identities.

## Push (alerts while the app is closed)

The app can only post notifications while it runs, and Android 15+ and iOS
stop a backgrounded app's network within seconds. So a device can hand the
gateway an Expo push token, and the gateway alerts through the Expo push
service (which hands on to Apple's APNs or Google's FCM) by itself. A device
can also (or instead) hand it an ntfy topic URL: the gateway then posts each
alert to that ntfy topic (ntfy.sh or a self-hosted server), and the free
ntfy app subscribed to it shows the alert. This needs no push setup in the
Rift app, so it works in builds without an Apple Developer Program account.

- `push` stores (or with `token: null` and `ntfy: null` removes) this
  device's registration in `devices.json`: the Expo half as
  `"push":{"token","long_ms","show_command","failures_only"}` (only with a
  token: the shape Rift builds before ntfy read, so a downgrade keeps
  working) and the ntfy half as
  `"ntfy":{"url","long_ms","show_command","failures_only"}` (a field older
  builds ignore). A file without `ntfy` has none.
  `failures_only` (default `false`) limits the `long_ms` alerts to commands
  that failed (an exit status other than 0); `notify` alerts either way.
  A gateway without the `push_failures` feature ignores the field, so the
  app then registers no `long_ms` with it rather than get alerts for
  successes. The token
  must look like `ExponentPushToken[…]` / `ExpoPushToken[…]` (URL-safe
  inside, at most 200 characters); `long_ms` is at most a day.
- `ntfy` (default `null`; send it only to a gateway with the `push_ntfy`
  feature) is `https://<host>[:port]/<topic>`: https only, a host name or
  IPv4 address, a topic of `[A-Za-z0-9_-]{1,64}`, no user, query or
  fragment, at most 200 characters. Never `localhost` / `*.localhost`, nor
  an IPv4 address that is loopback (127/8), 0/8, link-local (169.254/16),
  broadcast or multicast (224/4); a numeric last label must be a plain
  dotted quad (`127.1`, `2130706433`, `0x7f.0.0.1` are refused). Private
  LAN and 100.64/10 addresses are allowed. The app makes the topic
  `rift-<22 random base64url characters>`; the topic name is the secret
  (anyone who knows it can read the alerts). A bad token or URL refuses the
  whole message (`bad_request`) and keeps the previous registration.
- `notify` asks for one alert when the next command in `pane` finishes,
  whatever its length ("Notify me"); at most 32 panes per device, each
  forgotten after 24 h. `"off": true` (only to a gateway with
  `notify_off`; an older one would read it as a new wait) drops this
  device's wait on the pane: the bell switched off on a running block.
- For every `done` the gateway alerts each device with a token or an ntfy
  URL that asked about that pane (the wait is used up) or whose `long_ms`
  the command reached, at most 12 per device per minute (one alert counts
  once, through Expo, ntfy or both). While a device has a registration the
  gateway watches every pane even with nobody connected.
- The request is a POST of JSON to `https://exp.host/--/api/v2/push/send`
  (with `curl`, https only): `title` `✓ 0 · <command>` or
  `✕ N · <command>` (the command cut at 40 characters with `…`; `A command`
  unless `show_command`), `subtitle` `Finished in 113ms · <host>` or
  `Failed after 8.4s · <host>`, `data` `{"host":"<gateway key>","pane":N}` (a tap opens
  the pane), `priority` `high`, `sound` `default`. Output never goes out.
- A token the service answers with `DeviceNotRegistered` is removed (an
  ntfy URL next to it stays).
- An ntfy alert is a POST to the device's topic URL (with `curl`, https
  only, 5 s to connect and 15 s in all, the reply not read or logged), on a
  thread per device: a slow or dead ntfy server holds up no other device
  and no Expo post. While a device's post is under way its next alerts are
  dropped (logged), so one dead server costs at most one thread. The body is the
  Expo subtitle (`Finished in 113ms · <host>`); at most 4096 bytes (ntfy turns longer ones into attachments).
  Headers: `Title` the same words as the Expo title (as an RFC 2047
  `=?UTF-8?B?…?=` word when it is not printable ASCII, e.g. a host named
  `Jo’s Mac`), `Priority` `high` for a failed command else `default`, `Tags`
  `x` or `white_check_mark`, and `Click`
  `rift://host/<gateway key, percent-encoded>/pane/<pane id>`, the app's
  link to that pane. Output never goes out.
- No ntfy alert goes out about a pane the device follows right now (a live
  connection subscribed to it whose app is on screen and that was heard
  from within 20 s: the app pings every 15 s, and a dropped connection
  stays open until the 45 s idle timeout) unless the device asked for it
  (`notify`): the user is looking at it, and Rift cannot drop the ntfy
  app's banner. An Expo token still gets that alert (the app drops it in
  the foreground). With ntfy on, the app sends no token to a `push_ntfy`
  gateway: ntfy replaces Expo there (one alert per command).
- `visible` `{"on":bool}` says whether the app is on screen; a connection
  starts visible. The app sends `on: false` when it goes to the
  `background` (it keeps its socket for a while, but iOS suspends it, so it
  cannot notify; `inactive`, e.g. a Face ID prompt, is still on screen),
  `on: true` when it is `active` again, and after a `welcome` only when it
  is in the background. While not visible, the pane it
  follows gets ntfy alerts again. Sent only to a gateway whose `welcome`
  lists `push_ntfy` (the feature covers `push.ntfy` and `visible`); an
  older gateway never gets it.
- The app then posts no notification of its own for that computer (one alert
  per command), and drops an Expo alert about the pane on screen. With ntfy
  and no token, it notifies by itself about the pane it follows while it is
  on screen (the gateway sent nothing), by its own rules: not while that
  pane is shown.

## Phase 3: terminal screen, typed text, output paging

### Terminal screen

Full-screen programs (vim, less, htop, an ssh session's prompt) are not
command blocks. With `screen` `auto` or `on` the gateway sends the
subscribed pane's visible screen as text runs:

```json
{"t":"screen","pane":12,"cols":80,"rows":24,"alt":true,"cursor":[23,1],"full":true,"lines":[[0,[["top",null,null,1],[" - 10:02",null,"1e1e1e",0]]],[1,[]]]}
```

- `cols`, `rows`: the screen's size as sent (at most 300 × 120: a larger
  screen is cut to its top-left part). The phone sizes the pane only with
  `fit` (§ Phone-sized panes); without one it fits or scrolls the desktop's
  screen.
- On the alternate screen the rows are the program's grid. On the primary
  screen (`on`) they are Rift's own view of the pane, as the desktop draws
  it, bottom-anchored with the last `rows` lines, or as far up as the pane
  is scrolled (the desktop's wheel, a phone's `scroll`): each command's echoed
  prompt is one header row (`▾ cargo test  ~/rift git:(main)` at the left, the
  state badge `running`, `✓ 0` or `✕ 2` and the duration right-aligned; the
  command bold in the bright colour, the context dim; `▸` and `N lines
  hidden` for a collapsed block, whose output is left out), stray blank
  lines are left out, and the live prompt, which the desktop's editor owns,
  is not shown (nor the cursor, then). `cursor` is the row the program's
  cursor is on, or `null` when that line is not shown.
- `fg`, `bg`: the pane theme's default foreground and background
  (`"rrggbb"`), which runs with a `null` `fg` / `bg` are drawn in (the
  program's own default colours, reverse video already applied). Present on
  every frame, alternate screen included; absent from a gateway that does
  not send them (draw your own).
- `age_ms`: only on a phone-sized capture the program's output has moved on
  from (§ Phone-sized panes): how far behind it is, in whole seconds, from
  3000 on; absent otherwise.
- `scrollback`: on the primary screen, where its rows sit among the pane's
  numbered lines (§ Scrollback the phone scrolls); absent otherwise.
- `rules`: `[[row, "rrggbb"], …]`, every screen row that carries a block
  rule, the 2px bar the desktop draws left of a command block, header and
  output rows alike, and its colour: success green for exit 0, error red
  for a failure, the accent while the command runs, muted when no exit is
  known. Rows outside blocks (padding, the alternate screen) are not
  listed; the field is absent when no row has one. It is always the
  whole list, also in diff frames (`full: false`): a client replaces its
  rules with it; a change of rules alone (a command finished) is a frame
  with `lines: []`. A frame sent without styles (below) keeps `fg`, `bg`
  and `rules`.
- A header's state badge is a pill on the desktop: its run carries a `bg`,
  the badge colour at 13 % over the theme background.
- `alt`: the pane shows the alternate screen.
- `cursor`: `[row, column] while the program shows its cursor, else
  `null`.
- `lines`: `[row, runs]`, row 0 at the top. With `full: true` every row
  `0 … rows-1` is present (the first `screen` after `screen` `auto`/`on`
  or a re-`subscribe`, and whenever `cols` or `rows` change); otherwise only
  the rows that changed since the previous `screen`.
- A run is `[text, fg, bg, flags]`: `fg`/`bg` are the resolved colours
  (`"rrggbb"`, lowercase; the pane's theme and the program's palette
  changes applied, reverse video already swapped) or `null` for the
  default foreground / background; `flags` adds 1 bold, 2 italic, 4
  underline, 8 dim, 16 strikethrough. A wide character is one character of
  `text` that takes two columns. Blank cells in the default style at the
  end of a row are left out; an empty row is `[]`.
- A cell carries at most 16 combining marks (the rest are left out). A
  wide character in the last column sent (its second half cut off by the
  300-column limit) is sent as a space: a row never takes more than
  `cols` columns.
- A screen (or a diff) that would exceed 1 MiB is sent full without styles
  (every run `null`, `null`, `0`), each row's text cut so the message
  fits. Later diffs are against that unstyled screen: rows that differ
  from it come again with their styles.
- Screens come at most every 100 ms, only when something changed (a
  cursor move is a change). For 600 ms after a `scroll`, `keys` or `text`
  to the pane they come up to every 16 ms, so the program's redraw shows
  in a frame or two. Each diff is against the screen the phone was
  last sent; a phone that reads slower than the screen changes gets fewer
  screens (the newest, diffed against what it has), never a backlog.
- `screen_end` says screens stopped: mode `auto` and the pane left the
  alternate screen (also when the mode changes from `on` to `auto` while
  the pane is on the primary screen), or mode `off` was set. A new
  `subscribe` or `unsubscribe` stops them silently (and resets the mode
  to `off`).

### Phone-sized panes (feature `fit`)

A `fit` gives the phone a screen of its own size. While any client holds one
for a pane, the size wanted is the smaller of the desktop's own columns and
the phone's, and of its rows and the phone's. How the phone gets it depends on
the window:

- **In use** (it has focus and was typed in, clicked or scrolled within the
  last minute): the pane is never resized. A program on the alternate screen
  is told the phone's size for a moment (TIOCSWINSZ only, about 110 ms of a
  redraw) and what it draws goes to the phone alone; it is told its own size
  back and repaints the desktop's screen. Such a *capture* starts after 250 ms
  without output, at most every 2 s per pane, and only when the screen moved.
  A shell marker, leaving the alternate screen or typing ends one early. A
  program that does not redraw when told a size is not asked again for 60 s
  (and whatever it printed meanwhile reaches the desktop's screen). No capture
  yet, or none possible (the primary screen is never signalled, since what a
  program prints there is history): the phone gets the live frame at the
  desktop's size. A tap on a capture is refused, like one on a program that does not
  track the mouse: its cell is not the program's.
- **Left alone or not focused**: the pane's PTY takes that size, live, so
  programs such as Claude Code reflow to the phone's width; the desktop draws
  the smaller grid. Touching the window gives the pane its own size back.
  Any pane of any window: the active tab's, one in another tab (a tab never
  shown since it opened takes the size it opened at as its own) and one in a
  window that draws no frame (on another Space, minimized, under Settings, the
  display asleep) is sized when the fit arrives, not at the next frame.

A capture the program's output has moved on from carries `age_ms` (whole
seconds, from 3 s on: the phone says how old it is), and past 10 s gives way
to the live frame. A streaming program never pauses for 250 ms, so meanwhile the
phone has the last capture, then the live frame.

With
several phones the smallest columns and the smallest rows win. It counts
only while the client follows that pane: an `unsubscribe`, a `subscribe` to
another pane or a closed connection releases it, as does an explicit
release (`cols` and `rows` `null`). The
phone's height ignores its keyboard, which only covers the program's last rows
(they scroll into sight). The phone sends its `fit` again every 10
seconds while it holds one: the window drops a fit it did not hear again for
30 seconds, so a gateway that died never leaves the desktop shrunk. The
pane's size changes like any resize of the window's pane (the screen's
`cols` and `rows` follow, a `full` screen is sent).

### What programs copy (feature `clipboard`)

With the desktop's `remote_clipboard` on (off by default) what a program copies
with OSC 52, as far as `clipboard_access` lets it, also goes to the `operate`
devices connected, at most 48 KiB, within about half a second:

```json
{"t":"clipboard","text":"copied by the program"}
```

The phone puts it on its clipboard and says so. Off by default because a
program, or a host reached over ssh from it, could otherwise write to the
phone's clipboard without anyone asking.

### The desktop's theme (feature `theme`)

The welcome carries the theme the desktop draws with (`theme`, absent from
an older gateway), resolved as its windows show it (`theme`, or
`theme_light` while the system is light with `sync_os_theme` on; with no
window open, the configured `theme`). When it changes (Settings, a
`config.toml` edit, the system switching light or dark), every connection
gets a `theme` message within two seconds (the gateway reads the windows'
theme with the topology, then checks it every `THEME_POLL`, 1 s); one may repeat the theme its
welcome had.

```json
{"t":"welcome","v":2,"host":"studio-mac","os":"macos","version":"0.27.0","device":"d3f1","permission":"operate","features":["screen","text","output","clipboard","theme"],"theme":{"name":"Dark Default","dark":true,"bg":"050505","bg_chrome":"000000","surface":"1e1e1e","border":"ffffff1a","fg":"ffffff","text_bright":"ffffff","text_ui":"e6e6e6","text_dim":"9b9b9b","text_muted":"828282","accent":"19aad8","success":"1ca05a","error":"ff8272","warning":"e5a01a","selection":"76a7fa66","ansi":["616161","ff8272","b4fa72","fefdc2","a5d5fe","ff8ffd","d0d1fe","f1f1f1","8e8e8e","ffc4bd","d6fcb9","fefdd5","c1e3fe","ffb1fe","e5e6fe","feffff"]}}
{"t":"theme","theme":{"name":"Dracula","dark":true,"bg":"282a36",…,"ansi":[…16 colours]}}
```

A `Theme` is the theme's `name` (1 to 64 UTF-16 code units; the desktop
cuts a longer theme-file name to fit, ending it in "…", and sends "Custom" for an empty one), `dark` (its `bg`
is dark), the tokens `bg`, `bg_chrome`, `surface`, `border`, `fg`,
`text_bright`, `text_ui`, `text_dim`, `text_muted`, `accent`, `success`,
`error`, `warning`, `selection` as `rrggbb` or `rrggbbaa` (no `#`), and the
16 `ansi` colours, named as in the desktop's theme files
(docs/configuration.md, Themes). The app drops a malformed theme from a
welcome (the welcome still counts) and ignores a malformed `theme`
message. It makes its colours from the tokens (Settings › Appearance, Same
as the computer), holds each text colour to 4.5:1 on the grounds it draws it
on, and keeps the last theme a computer sent for one that sends none.

### Output paging

`Block.output` is the tail of a block's output. Earlier lines are fetched
on request:

```json
{"t":"output","id":"o1","pane":12,"block":86,"from":0,"limit":500}
{"t":"output","id":"o1","pane":12,"block":86,"from":0,"text":"…","total":40}
```

- Lines are numbered from 0, the block's first output line, the way
  `dropped` counts them (soft-wrapped rows joined; a row gone from
  scrollback is one line).
- The answer holds lines from `from` on, at most `limit` (1 … 500) lines
  and 64 KiB, cut at the end of a line (a single line longer than 64 KiB
  keeps its first 64 KiB). Its `from` is the first line it holds: when the
  requested lines are already gone from scrollback it is the first line
  still held (and `text` may be empty). `text` ends with a line break
  unless it reaches the block's unfinished last line.
- `total`: the block's output lines so far (a running block grows).
- `from` at or past `total` gives an empty `text`.
- An unknown block (or one being typed at the prompt) answers `ack` with
  `error` `no_block`; an unknown pane `no_pane`; past the rate `rate`.
  `view` devices may page.

### Scrollback the phone scrolls (feature `history`)

On the primary screen (`screen` `on`) the phone scrolls the lines above
Rift's view of the pane itself, with its own scrolling, instead of sending
`scroll` for every step of a drag. Each styled `screen` carries
`scrollback`:

```json
{"t":"screen","pane":12,"cols":80,"rows":24,"alt":false,"cursor":[3,2],"full":false,"lines":[[23,[["hello",null,null,0]]]],"scrollback":{"epoch":7,"first":120,"top":400}}
```

- Every line of the view (the display list `screen` draws: block headers,
  output, padding) has a number. Screen row `i` is line `top + i`; `first`
  is the oldest line the computer still holds.
- A line keeps its number while the numbering (`epoch`) lasts: new lines
  come at the end, trimmed scrollback takes the oldest away (`first`
  grows). Any other change above the screen (a block folded or unfolded,
  the width reflowed) starts a new numbering; its epoch was never given
  out before, also by an earlier run of Rift. A change of `scrollback`
  alone is a frame with `lines: []`.
- The alternate screen, a phone-sized capture and a shell kept while Rift
  is closed have no `scrollback`.

Lines above the screen come on request:

```json
{"t":"history","id":"h1","pane":12,"epoch":7,"from":340,"to":400}
{"t":"history","id":"h1","pane":12,"epoch":7,"first":120,"from":398,"lines":[[["▾ cargo test","e6e6e6",null,1]],[["running 12 tests",null,null,0]]],"rules":[[0,"3fb950"]]}
```

- The answer holds lines `from` … `to - 1` that the computer still holds,
  styled as `screen` rows, at most 200 (a longer range keeps its end, the
  lines nearest the screen) and at most 1 MiB (lines leave from its start
  until it fits). Its `from` is the number of `lines[0]`. `rules`:
  `[[index, "rrggbb"], …]`, the lines carrying a block rule, by their
  index in `lines`.
- Another `epoch` than the one asked for answers with the current one and
  no `lines`: the phone's lines are out of date; it starts again from the
  next `screen`.
- `from` must be below `to` (else `ack` `bad_request`); an unknown pane
  answers `no_pane`; past the rate `rate`. `view` devices may read it.

### A full-screen program stitched into one page (feature `stitch`)

A program on the alternate screen draws only the rows in view: what is
above or below exists nowhere until it draws it. With `stitch`, the gateway
scrolls the program itself (a local round trip per wheel step), reads each
screen once it rests, and joins the screens into one page the phone
scrolls natively.

```json
{"t":"page_ahead","id":"g1","pane":12,"epoch":0,"dir":"up","rows":72}
{"t":"page_seek","id":"g2","pane":12,"epoch":3,"at":-40}
{"t":"page","pane":12,"epoch":3,"area":[1,37,0,81],"top":-40,"bottom":36,"at":-40,"from":-40,"lines":[[["❯ question 40",null,null,1]]],"ends":[true,false],"state":"ok"}
```

- `page_ahead` asks for up to `rows` (1 to 500) more rows past the page's end
  in `dir`; `page_seek` moves the program so that page row `at` is the first
  row of its area. Each is acked at once (`busy` while a job runs on the
  pane, `forbidden` from a `view` device, `rate` past 5 a second), then
  answered with `page` messages: the whole page when it is new, else the rows
  it added, at most 200 rows each (`from`, `lines`); a seek carries no rows.
- `epoch` 0, or one the gateway no longer holds, or a screen that shows no
  part of the page (the program moved, or drew something else) starts a new
  page from the screen as it is, with a new `epoch`.
- Page rows are numbered from the screen the page started from (its first
  area row is 0, rows above it negative); `top..bottom` is the page, `at`
  the row the program shows first. `area` is the scrolling rectangle: screen
  rows `[0]..[1]`, columns `[2]..[3]`; `lines` are that rectangle's cells.
- How the gateway joins screens (`crates/rift-remote/src/stitch.rs`): it
  scrolls 3 wheel lines a step at first, then past the page's end as many as
  bring about a third of the area (from the last step's rows a line, growing
  by half at most a step, at most 2/5 of the area past the overlap: Claude
  Code moves up to 2.5 rows a line under a repeated wheel); more inside rows
  the page already holds. It
  reads the screen once it has not changed for 30 ms (Claude Code redraws a
  scroll in pieces; a step it does not answer within 150 ms moved nothing).
  From the first 8 screens it learns the area: the rows whose text moves
  from one screen to the next (a sticky header, a "jump to bottom" hint, a
  status line change in place and stay out) and the columns that change,
  widened to the screen's edges or a pane border (`│`). A screen joins the
  page only past an overlap of at least 4 rows with the page's end, or is
  found inside the page; it never adds wrong rows. A screen too far to join
  (Claude Code jumps tens or hundreds of rows now and then, laying a long
  message out anew) sends the program back in steps that double (at most a
  quarter of the page) until a screen joins or is found, then on at half the
  step that went too far; after 4 such jumps in a job, or a screen caught mid-redraw that never
  joins, the page stops growing (`stopped`).
- `stopped` is not the end: the phone asks again after 1.5 s (on its own
  twice in a row while the view is near that edge, again from each drag). A seek that
  finds rows the page does not hold (the program laid them out anew since)
  answers `stopped` too; the next `page_ahead` then starts a new page there.
- `ends`: the program's start and end were reached (two steps moved nothing
  in the area: a status line or a clock changing beside it does not count).
  `state` `flat`: nothing on the screen scrolls like a document (htop, a
  menu), so there is no page; the phone keeps the wheel (`scroll`).
- While a job runs the pane's screens wait (none go out); afterwards the
  screen shows where the program is now. A job stops after 3 s, and a page
  at 5000 rows.

### Prompt chip menus (feature `chips`)

What the desktop's prompt chips open, for the pane's chips on the phone:
the folder chip's directories, the branch chip's local branches, and the
code review panel's changed files with each file's diff.

```json
{"t":"dirs","id":"m1","pane":12}
{"t":"dirs","id":"m1","pane":12,"cwd":"/Users/me/rift","parent":true,"dirs":["crates","docs","mobile"],"more":false}
{"t":"branches","id":"m2","pane":12}
{"t":"branches","id":"m2","pane":12,"current":"main","branches":["feat/chips","main"],"more":false}
{"t":"changes","id":"m3","pane":12}
{"t":"changes","id":"m3","pane":12,"branch":"main","files":[{"path":"docs/ui.md","status":"A","added":12,"removed":0,"staged":true}],"more":false}
{"t":"diff","id":"m4","pane":12,"path":"docs/ui.md","staged":true}
{"t":"diff","id":"m4","pane":12,"path":"docs/ui.md","staged":true,"binary":false,"truncated":false,"lines":[["@","@@ -0,0 +1,12 @@"],["+","# UI"]]}
```

- `dirs`: the pane's directory (`cwd`, null when unknown), whether it has a
  parent, and its sub-directories by name, sorted case-insensitively,
  hidden ones (a leading `.`) left out: what the desktop's folder menu
  lists. Picking one runs `cd ..` or `cd ./<name>` (shell-quoted) as an
  ordinary `run`.
- `branches`: the repository's local branches, the checked-out one
  (`current`; a short commit id when detached) first, then by name; empty
  and null outside a repository; a branch whose name starts with `-` is
  left out (`git checkout -x` would read it as an option). Picking one
  runs `git checkout <name>`, a new name `git checkout -b <name>`
  (shell-quoted), as the desktop's branch menu does; both offer a new name
  only when it does not start with `-` and has none of whitespace,
  `~ ^ : ? * [ \`.
- `changes`: the code review panel's files: `staged` ones (HEAD → index)
  first, then the working tree's (untracked included), each with its
  repository-relative `path`, `status` (`M`, `A`, `D`, `R`, `C`, `T`, `?`
  untracked, `U` conflicted), `added` and `removed` lines. `branch` is null
  outside a repository (then `files` is empty).
- `diff`: one file of `changes` (`path` and `staged` as listed): its lines
  in order as `[kind, text]`, kind `@` a hunk header, `" "` context, `+`
  added, `-` removed. `binary` files have no lines. A path that is not
  among the changes answers `ack` `no_block`; `path` is matched
  literally, never as a pattern. An untracked file's diff (its whole
  content) answers `view` devices `ack` `forbidden`.
- Bounds: at most 500 `dirs`, `branches` or `files` (`more`: some were
  left out), names and paths of at most 1024 bytes (longer ones are left
  out), and a `diff` of at most 2,000 lines and 256 KiB, each line cut at
  1,000 characters (`truncated`: something was cut). A request's `path`
  is at most 1,024 bytes (the longest path `changes` lists). The four requests share `output`'s rate.
- An unknown pane answers `ack` with `error` `no_pane`. `view` devices
  may read the lists and tracked files' diffs; only `operate` devices can
  run the commands they lead to. The names come from the computer's file system and repository:
  the app shell-quotes them and never runs one unquoted.

### Pull request, interpreter, Docker and Time on tab menus (feature `prompt_chips`)

What the rest of the desktop's chips open. The desktop answers from the same
code its own chip menus use (`crates/rift-terminal/src/views/chip_menu.rs`);
the phone runs a row's command through `run`, so Settings › Ask before
running and view-only devices apply as for any command.

```json
{"t":"chip","id":"m7","pane":12,"kind":"pr"}
{"t":"chip","id":"m7","pane":12,"kind":"pr","number":42,"state":"open","url":"https://github.com/me/flow/pull/42","more":false}
{"t":"chip","id":"m8","pane":12,"kind":"python"}
{"t":"chip","id":"m8","pane":12,"kind":"python","current":"3.12.4","versions":[{"version":"3.12.4","source":"pyenv","current":true,"command":"rift-use pyenv '3.12.4'"}],"more":false}
{"t":"chip","id":"m5","pane":12,"kind":"docker"}
{"t":"chip","id":"m5","pane":12,"kind":"docker","project":"flow","error":null,"containers":[{"service":"api","state":"running","running":true,"ports":["8080→8080"],"uptime":"2 hours","health":null,"logs":"docker compose --project-directory '/Users/me/flow' logs --follow --tail 200 'api'"}],"more":false}
{"t":"chip","id":"m6","pane":12,"kind":"elapsed"}
{"t":"chip","id":"m6","pane":12,"kind":"elapsed","seconds":23580,"counting":true,"line":"Started 09:15 am","more":false}
```

- `pr`: the pull request of the checked-out branch (`state` `open`, `draft`,
  `merged` or `closed`; the desktop's `gh` lookup). The phone opens `url` in
  its browser, `https` only.
- `python`, `php`: the installed interpreters as the desktop's menu lists
  them, newest first, with the one the pane's shell runs (`current`).
  `command` is the desktop menu's own `rift-use …` line for the row; the
  phone runs it as a command in the pane.
- `docker`: the compose project's containers, running ones first: `state`
  (`running`, `exited`, …), `ports` as the desktop's table prints them
  (`3306→3306`, `/udp` only for UDP), `uptime` (`2 hours`, or the whole
  status when stopped), `health` (`healthy`, `starting`, `unhealthy` or
  null). `logs` is the command the row runs, following the service's logs.
  `error` says why there are none (Docker is not running). The window asks
  Docker for fresh state after each answer.
- `elapsed`: the Time on tab chip's card. `seconds` and `counting` are the
  chip's clock, `line` its hover line (`Started 03:48 pm`, or `Counted 2h 05m
  · running now` when it counts running commands only). `reset_elapsed`
  restarts it.
- The phone runs only commands of the two shapes these menus build
  (`rift-use <php|python|pyenv> '<path or version>'` and `docker compose
  --project-directory <dir> logs --follow --tail 200 <service>`).
- A chip with nothing to open (no pull request, no compose project, no
  clock yet) answers `ack` with `error` `no_pane` and a message saying so
  (`pane 12 unavailable: no pull request`), as the other menus do when the
  window cannot answer. Bounds,
  rate and errors are the other chip menus' (`more` is true when entries
  were left out); `view` devices may read these menus.

### Completion and history (feature `complete`)

What the desktop's command line would complete, and what ran in the pane
before, for the phone's Run sheet and the pane's suggestions:

```json
{"t":"complete","id":"k1","pane":12,"line":"cargo t","cursor":7}
{"t":"complete","id":"k1","pane":12,"items":[{"text":"cargo test","kind":"subcommand"},{"text":"cargo tree","kind":"subcommand"}],"history":[{"text":"cargo test --workspace","at":1760364000},{"text":"cargo test -p rift-remote","at":null}],"more":false}
```

- `items`: the desktop editor's completions for the word at `cursor` (a
  byte offset into `line`, at a character boundary), each as the whole
  resulting command line (`text`) and its `kind`: `command`,
  `subcommand`, `option`, `path`, `history` or `value`.
- `history`: commands starting with `line`, newest first and each once:
  the pane's blocks (`at`: when it finished, Unix seconds; null while it
  runs), then the pane's command-line history and the shell history the
  desktop imported (`at` null). Lines the desktop's ghost-text ✕ ignored
  (`ignored-suggestions.json`) are left out, as from its own suggestions.
- Bounds: `line` at most 1,024 bytes; at most 50 `items` and 50
  `history` entries (`more`: some were left out), each `text` at most
  1,024 bytes. It shares `output`'s rate. An unknown pane answers `ack`
  `no_pane`; a `cursor` outside `line` or inside a character
  `bad_request`; a `view` device `forbidden` (history can hold secrets
  typed on a command line, and path completion lists any folder).

## Connection history and permissions (Phase 2)

- `<data dir>/remote/history.jsonl` (0600): one line per closed
  connection, `{"device","name","via":"lan"|"relay","address","start","end","reason"}`
  (times RFC 3339, whole seconds), 500 kept. Failed handshakes are
  recorded with `device` and `name` null; when the file is full they are
  dropped first (oldest first), so a flood of refused attempts never pushes
  out a paired device's connections. `reason` is one of `closed`,
  `error`, `idle`, `too_slow`, `too_large`, `bad_request`, `revoked`,
  `permission`, `forgotten`, `stopped`, or for a failed handshake
  `handshake`, `version`, `auth`, `busy`.
- Through the relay the gateway cannot see the phone's address: `address`
  is the relay's `host:port`, and the per-address failed-handshake limit
  does not apply there (the relay's per-address `/connect` limit does).
  Someone who knows a routing id can occupy that gateway's relay sockets
  (within the relay's per-address limits) and slow relay phones down; LAN
  phones are unaffected (separate budget).
- A device's permission can be changed on the desktop (Settings › Remote
  access, or `rift-terminal remote permission ID operate|view`); open
  connections of that device close within a second so the next handshake
  carries the new permission. The gateway reads `devices.json` at every
  handshake and checks its file stamp before each operation requiring `operate`
  (including `run`, `keys`, `text`, uploads, shell actions and completions),
  re-reading it when it changed. A lowered permission applies at once, even
  before the open connections close; missing or unreadable permission data
  refuses the operation.

## Desktop side

| Path (under `<data dir>/remote/`, directory 0700, files 0600) | Holds |
|---|---|
| `gateway.key` | The gateway's static X25519 private key (32 bytes), created once |
| `devices.json` | `[{"id","name","platform","key","permission","paired_at","last_seen","push"?,"ntfy"?}]` (`key` base64url, times RFC 3339, `push` and `ntfy` see § Push) |
| `pairing.json` | Live pairing secrets: `[{"hash","permission","expires_at"}]` (SHA-256 of the secret, never the secret) |
| `status.json` | Written by the running gateway: `{"pid","addresses":[...],"clients":[{"device","name","since","via"}],"relay"?:{"url","state","error"?,"latency_ms"?,"retry_at_ms"?}}` (`relay` only while one is configured; `latency_ms` is the last ping's round trip to the relay while connected, `retry_at_ms` the next try's Unix time while `error`); removed when it stops |
| `relay-retry` | Empty file a window writes (Settings › Remote access › Try again); the relay connection removes it and retries at once instead of after its backoff |
| `relay.json` | `{"secret":"<32 bytes, base64url>"}`: the gateway's `/accept` secret at the relay |
| `history.jsonl` | Connection history (see above) |
| `uploads/` | Images and files phones pasted (`upload`), as `rift-paste-<unix ms>-<8 random hex>.<ext>` (`.png` or `.jpg` for an image), each 0600; a name the gateway makes, never one the phone sent. Files older than 24 hours are removed at the next upload and when the gateway starts |
| `gateway.lock` | Held (flock) by the running gateway; one gateway per user |

- `remote_access = false` (config, default) keeps the gateway stopped. When
  it is `true`, every Rift window makes sure a gateway runs
  (`rift-terminal remote --managed`, detached); the gateway exits within 2
  seconds after `remote_access` becomes `false`. On macOS with durable
  sessions it stays running after the last window closes, with a menu bar
  icon, until *Quit Rift Completely* or Remote access is turned off.
  Otherwise it exits 30 seconds after the last Rift window is gone.
- A gateway counts as running only while `gateway.lock` is held; a stale
  `status.json` is ignored. A window starts one at most every 5 seconds.
- The gateway reads windows through their control sockets: `status`
  (with the window's resolved `theme`, as the welcome carries it),
  `blocks` (read-only; `"meta": true` leaves the output out, which the
  `done` probe uses), `screen` (`{"cmd":"screen","pane":12,"force":false}`:
  the pane's screen, or none when it is not on the alternate screen and
  `force` is false, with the pane's output `generation`, which every
  parsed chunk of output, resize, clear and theme change moves; with
  `"since":N` and the generation still `N` the answer is
  `{"ok":true,"generation":N,"unchanged":true}` and no screen is built, so
  the gateway's 100 ms reads of an idle pane cost the window nothing) and
  `output`
  (`{"cmd":"output","pane":12,"block":86,"from":0,"limit":500}`), `history`
  (`{"cmd":"history","pane":12,"epoch":7,"from":340,"to":400}`:
  `{"ok":true,"pane":12,"history":{"epoch","first","from","lines","rules"}}`), `menu`
  (`{"cmd":"menu","pane":12,"kind":"dirs"}`, `kind` also `branches` or
  `changes`: `{"ok":true,"pane":12,"menu":{…}}`), `diff`
  (`{"cmd":"diff","pane":12,"path":"docs/ui.md","staged":true}`:
  `{"ok":true,"pane":12,"diff":{"binary","truncated","untracked","lines"}}`,
  or the error `no such change`), `complete`
  (`{"cmd":"complete","pane":12,"line":"cargo t","cursor":7}`; these three
  read the file system and Git, so a thread of the window's answers them,
  never its UI thread; the pane's `waiting` and `running` come with
  `status`), and
  `remote-input`
  (`{"cmd":"remote-input","pane":12,"device":"Sam's iPhone","command":"…"}`,
  `{…,"keys":["ctrl+c"]}` or `{…,"text":":wq"}`), which a window accepts only while
  `remote_access` is `true`, independent of `control_api`. Its errors:
  `remote access is off`, `no such pane`, `busy` (the pane is running
  something, or a command was submitted and has not started yet; for
  `text`: the pane is at its prompt, or a command was submitted and has
  not started yet), `the command line has text; send the command with run`
  (`keys` with `enter` at a prompt with a draft; the gateway's `busy`),
  `bad request: …`. `remote-fit`
  (`{"cmd":"remote-fit","pane":12,"cols":64,"rows":40}`, or both `null` to
  release; the smallest of the clients' fits, sent again on every refresh
  of one) has the same gate and the errors `remote access is off`, `no such
  pane` and `bad request: …`; the window sizes the pane to the smaller of its
  own grid and the fit until the release or 30 s without a refresh.
  `remote-open`
  (`{"cmd":"remote-open","pane":12,"device":"Sam's iPhone","kind":"tab"}`,
  answering the new pane as `pane`) has the same gate and the errors
  `remote access is off`, `no such pane` and `cannot split this pane`.
  `remote-close`
  (`{"cmd":"remote-close","pane":12,"device":"Sam's iPhone","kind":"pane"}`,
  `kind` `pane` or `tab`; no confirmation, answering `ok`) has the same gate
  and the errors `remote access is off` and `no such pane`.
  `remote-paste` (`{"cmd":"remote-paste","pane":12,"device":"Sam's iPhone","text":"…"}`),
  `remote-mouse` (`{"cmd":"remote-mouse","pane":12,"col":4,"row":7}`) and
  `remote-clipboard` (`{"cmd":"remote-clipboard","since":3}`, answered with
  `clipboard_seq` and `clipboard` when a newer copy exists) carry those messages.
  `remote-scroll`
  (`{"cmd":"remote-scroll","pane":12,"lines":-3,"operate":false}`; `operate`
  whether the device may send the program input, false when left out) has
  the same gate and the errors `remote access is off`, `no such pane`,
  `bad request: …` (`lines` 0 or past 50) and `view only: the wheel goes to
  the program` (the gateway's `forbidden`). A view move changes the
  phones' own view of the pane (the one `screen` builds), never the
  desktop's `view_offset`. `remote-upload`
  (`{"cmd":"remote-upload","pane":12,"device":"Sam's iPhone","name":"rift-paste-1760364000123-0a1b2c3d.png"}`)
  pastes the path of `uploads/<name>`, a file the gateway saved: the window
  takes only a name of that form (`rift-paste-<ms>-<8 hex>.<ext>`, `ext` 1 to 8 of `a-z0-9`) and a
  plain file (not a link) there, else `bad request: …`; the same gate, and
  `busy` where `text` is busy. The gateway removes the file again when the
  window refuses it. With `"check":true` (no `name`) it only answers
  whether the pane takes an image now; the gateway asks so at an upload's
  first chunk.
- A request whose caller stopped waiting (the socket answers `window did
  not answer` after 5 s) is dropped when the window reaches it, never
  carried out late.
- Every window shows a tab-bar chip while at least one device is connected
  (from `status.json`); clicking it opens Settings › Remote access. After
  every `status.json` change the gateway sends `{"cmd":"remote-status"}`
  (best effort, 500 ms timeout, same level as `status`) to each window, which
  re-reads the file and redraws, so an idle window updates within a second.
