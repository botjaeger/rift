/// Streaming scanner for the OSC sequences Rift cares about:
/// OSC 133 (shell-integration block markers), OSC 7 (cwd report) and the
/// others [`Marker`] lists.
/// Bytes are NOT consumed — the same stream is fed to the VT parser; this
/// scanner only spots marker positions so the caller can split the stream
/// and record grid state at each marker boundary.
use crate::shell_widget::WidgetMsg;

#[derive(Debug, Clone, PartialEq)]
pub enum Marker {
    /// OSC 133;A — start of prompt.
    PromptStart,
    /// OSC 133;B — end of prompt, user input begins.
    CommandStart,
    /// OSC 133;C — command accepted, output begins. `command` is the
    /// command line the shell read (`133;C;cmdline_url=<percent-encoded>`,
    /// kitty's form; Rift's hooks send it): exact where the screen's echo is
    /// not, e.g. replayed at another width. `None` when absent or too long.
    OutputStart { command: Option<String> },
    /// OSC 133;D;`<exit>` — command finished.
    Done { exit: Option<i32> },
    /// OSC 7;file://host/path — working directory changed. `host` is empty
    /// when the sequence names none; see [`host_is_local`].
    Cwd { path: String, host: String },
    /// APC `G…` — kitty graphics command (the bytes after `G`).
    Graphics(Vec<u8>),
    /// OSC 1337;File=… — iTerm2 inline image (the bytes after `File=`).
    InlineImage(Vec<u8>),
    /// A desktop-notification request: OSC 9 (iTerm2/ConEmu), OSC 777
    /// `notify` (urxvt/VTE) or OSC 99 (kitty).
    Notify { title: String, body: String },
    /// OSC 1337;SetUserVar=`name`=`base64 value` (iTerm2): shell state the
    /// prompt chips show (Python environment, php/python3 paths) and the
    /// shell's `$PATH` for command completion. Bounded, control characters
    /// dropped.
    UserVar { name: String, value: String },
    /// OSC 6973 — Rift's shell-widget hand-off (`shell/rift.*`; see
    /// [`crate::shell_widget`]).
    ShellWidget(WidgetMsg),
    /// OSC 9;4;`st`;`pr` (ConEmu / Windows Terminal) — taskbar progress.
    /// `None` removes it (`st` 0).
    Progress(Option<Progress>),
}

/// The kind of an OSC 9;4 progress report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    /// `st` 1: a percentage.
    Normal,
    /// `st` 2: failed (the percentage optional).
    Error,
    /// `st` 3: busy, no percentage.
    Indeterminate,
    /// `st` 4: paused / warning (the percentage optional).
    Paused,
}

impl ProgressState {
    /// The name `dump` and the docs use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Error => "error",
            Self::Indeterminate => "indeterminate",
            Self::Paused => "paused",
        }
    }
}

/// One OSC 9;4 progress report. `pct` is 0–100 (clamped); `None` when the
/// sequence gave none (always for indeterminate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub state: ProgressState,
    pub pct: Option<u8>,
}

/// `st[;pr]` of OSC 9;4. Anything else (an unknown state, a non-numeric or
/// out-of-range percentage, extra fields) is malformed: `None`.
fn parse_progress(rest: &str) -> Option<Marker> {
    let mut parts = rest.split(';');
    let st = parts.next()?;
    let pr = parts.next().filter(|p| !p.is_empty());
    if parts.next().is_some() {
        return None;
    }
    // digits only (`parse` would also take a sign); the text is bounded by
    // MAX_OSC, and a value past u32 fails to parse
    let pct = match pr {
        Some(p) if p.bytes().all(|b| b.is_ascii_digit()) => {
            Some(p.parse::<u32>().ok()?.min(100) as u8)
        }
        Some(_) => return None,
        None => None,
    };
    let state = match st {
        "0" => return Some(Marker::Progress(None)),
        "1" => ProgressState::Normal,
        "2" => ProgressState::Error,
        "3" => ProgressState::Indeterminate,
        "4" => ProgressState::Paused,
        _ => return None,
    };
    let pct = match state {
        // a normal report without a value starts at 0 (Windows Terminal)
        ProgressState::Normal => Some(pct.unwrap_or(0)),
        ProgressState::Indeterminate => None,
        _ => pct,
    };
    Some(Marker::Progress(Some(Progress { state, pct })))
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Ground,
    Esc,
    Osc,
    /// ESC seen inside an OSC string (possible ST terminator).
    OscEsc,
    /// First byte of an APC string (`ESC _ … ESC \`): `G` = kitty graphics.
    ApcStart,
    /// Body of a kitty graphics APC string.
    Apc,
    /// Body of any other APC string (skipped; a stray `G` inside is data).
    ApcSkip,
    ApcEsc,
}

/// CAN and SUB abort any control string.
const fn is_abort(b: u8) -> bool {
    b == 0x18 || b == 0x1a
}

const MAX_OSC: usize = 4096;
/// `133;C` may carry the command line (a long pipeline, a pasted script).
const MAX_CMDLINE: usize = 64 << 10;
const OUTPUT_START: &[u8] = b"133;C";
/// The user variable carrying the shell's `$PATH` (see `shell/rift.*`).
pub const USER_VAR_PATH: &str = "RiftPath";
/// Inline images and graphics commands carry base64 payloads.
const MAX_IMAGE: usize = 48 << 20;
const ITERM_FILE: &[u8] = b"1337;File=";
/// A shell widget's report carries a whole command line (percent-encoded).
const WIDGET_LINE: &[u8] = b"6973;W;";
const MAX_WIDGET_LINE: usize = 64 << 10;

/// A piece of the input chunk: bytes `range` end either at a marker boundary
/// (marker fires AFTER those bytes are parsed) or at the chunk end.
#[derive(Debug)]
pub struct Segment {
    pub end: usize,
    pub marker: Option<Marker>,
}

#[derive(Default)]
pub struct OscScanner {
    state: State,
    buf: Vec<u8>,
    /// The OSC string outgrew its bound: its text is cut, so it is not
    /// reported (a truncated OSC 7 path would name another directory).
    clipped: bool,
}

impl OscScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// The byte after an ESC: starts an OSC or APC string, or is not ours.
    fn after_esc(&mut self, b: u8) {
        self.buf.clear();
        self.clipped = false;
        self.state = match b {
            b']' => State::Osc,
            b'_' => State::ApcStart,
            0x1b => State::Esc,
            _ => State::Ground,
        };
    }

    /// Split `chunk` at marker boundaries. Segment ends are cumulative
    /// offsets into `chunk`; the last segment always reaches `chunk.len()`.
    pub fn scan(&mut self, chunk: &[u8]) -> Vec<Segment> {
        let mut segments = Vec::new();
        for (i, &b) in chunk.iter().enumerate() {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Esc;
                    }
                }
                State::Esc => self.after_esc(b),
                State::ApcStart => {
                    if b == 0x1b {
                        self.state = State::ApcEsc;
                    } else if is_abort(b) {
                        self.state = State::Ground;
                    } else if b == b'G' {
                        // only kitty graphics (`G`) bodies are kept
                        self.buf.push(b);
                        self.state = State::Apc;
                    } else {
                        self.state = State::ApcSkip;
                    }
                }
                State::Apc => {
                    if b == 0x1b {
                        self.state = State::ApcEsc;
                    } else if is_abort(b) {
                        self.state = State::Ground;
                        self.buf.clear();
                    } else if self.buf.len() < MAX_IMAGE {
                        self.buf.push(b);
                    }
                }
                State::ApcSkip => {
                    if b == 0x1b {
                        self.state = State::ApcEsc;
                    } else if is_abort(b) {
                        self.state = State::Ground;
                    }
                }
                State::ApcEsc => {
                    if b == b'\\' {
                        self.state = State::Ground;
                        if self.buf.first() == Some(&b'G') && self.buf.len() < MAX_IMAGE {
                            segments.push(Segment {
                                end: i + 1,
                                marker: Some(Marker::Graphics(self.buf[1..].to_vec())),
                            });
                        }
                        self.buf.clear();
                    } else {
                        // ESC inside the string starts a new sequence: the
                        // unterminated one is dropped, this byte is re-read
                        self.after_esc(b);
                    }
                }
                State::Osc => {
                    if is_abort(b) {
                        self.state = State::Ground;
                        self.buf.clear();
                    } else if b == 0x07 {
                        self.state = State::Ground;
                        if let Some(marker) = self.finish_osc() {
                            segments.push(Segment {
                                end: i + 1,
                                marker: Some(marker),
                            });
                        }
                        self.buf.clear();
                    } else if b == 0x1b {
                        self.state = State::OscEsc;
                    } else if self.buf.len() < MAX_OSC
                        || (self.buf.starts_with(ITERM_FILE) && self.buf.len() < MAX_IMAGE)
                        || (self.buf.starts_with(WIDGET_LINE) && self.buf.len() < MAX_WIDGET_LINE)
                        || (self.buf.starts_with(OUTPUT_START) && self.buf.len() < MAX_CMDLINE)
                    {
                        self.buf.push(b);
                    } else {
                        self.clipped = true;
                    }
                }
                State::OscEsc => {
                    if b == b'\\' {
                        self.state = State::Ground;
                        if let Some(marker) = self.finish_osc() {
                            segments.push(Segment {
                                end: i + 1,
                                marker: Some(marker),
                            });
                        }
                        self.buf.clear();
                    } else {
                        // ESC inside the string starts a new sequence (the
                        // unterminated OSC is dropped, as vte does)
                        self.after_esc(b);
                    }
                }
            }
        }
        segments.push(Segment {
            end: chunk.len(),
            marker: None,
        });
        segments
    }
}

impl OscScanner {
    /// The marker of the OSC string just terminated. A clipped string is
    /// dropped, except a `133;C` whose command line was too long (the
    /// command still started) and a shell widget's lost report (it still
    /// ends the hand-off waiting for it).
    fn finish_osc(&mut self) -> Option<Marker> {
        if std::mem::take(&mut self.clipped) {
            if self.buf.starts_with(WIDGET_LINE) {
                return Some(Marker::ShellWidget(WidgetMsg::Dropped));
            }
            let c = self.buf.strip_prefix(OUTPUT_START)?;
            return matches!(c.first(), None | Some(b';'))
                .then_some(Marker::OutputStart { command: None });
        }
        parse_osc(&self.buf)
    }
}

/// The command line of a `133;C` (its parameters after `C`): `cmdline_url`,
/// percent-decoded, without control characters other than newline and tab,
/// and without the blanks around it (zsh hands over ` cmd` as typed).
fn cmdline<'a>(mut params: impl Iterator<Item = &'a str>) -> Option<String> {
    let url = params.find_map(|p| p.strip_prefix("cmdline_url="))?;
    let text: String = percent_decode(url)
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn parse_osc(buf: &[u8]) -> Option<Marker> {
    if let Some(rest) = buf.strip_prefix(ITERM_FILE) {
        return (buf.len() < MAX_IMAGE).then(|| Marker::InlineImage(rest.to_vec()));
    }
    // the command line in a C may carry any bytes: the block still starts
    if let Some(rest) = buf.strip_prefix(b"133;C;")
        && std::str::from_utf8(buf).is_err()
    {
        let lossy = String::from_utf8_lossy(rest);
        return Some(Marker::OutputStart {
            command: cmdline(lossy.split(';')),
        });
    }
    let s = std::str::from_utf8(buf).ok()?;
    if let Some(rest) = s.strip_prefix("6973;") {
        return crate::shell_widget::parse(rest).map(Marker::ShellWidget);
    }
    if let Some(rest) = s.strip_prefix("133;") {
        let mut parts = rest.split(';');
        return match parts.next()? {
            "A" => Some(Marker::PromptStart),
            "B" => Some(Marker::CommandStart),
            "C" => Some(Marker::OutputStart {
                command: cmdline(parts),
            }),
            "D" => Some(Marker::Done {
                exit: parts.next().and_then(|p| p.parse().ok()),
            }),
            _ => None,
        };
    }
    // notifications: text only, bounded
    let clip = |t: &str| -> String { t.chars().filter(|c| !c.is_control()).take(256).collect() };
    if let Some(rest) = s.strip_prefix("9;") {
        // `9;4;…` is ConEmu progress, not a notification
        if let Some(progress) = rest.strip_prefix("4;") {
            return parse_progress(progress);
        }
        if rest.is_empty() {
            return None;
        }
        return Some(Marker::Notify {
            title: clip(rest),
            body: String::new(),
        });
    }
    if let Some(rest) = s.strip_prefix("777;notify;") {
        let (title, body) = rest.split_once(';').unwrap_or((rest, ""));
        return Some(Marker::Notify {
            title: clip(title),
            body: clip(body),
        });
    }
    if let Some(rest) = s.strip_prefix("99;") {
        let (meta, payload) = rest.split_once(';')?;
        if payload.is_empty() {
            return None;
        }
        let is_body = meta.split(':').any(|kv| kv == "p=body");
        return Some(if is_body {
            Marker::Notify {
                title: String::new(),
                body: clip(payload),
            }
        } else {
            Marker::Notify {
                title: clip(payload),
                body: String::new(),
            }
        });
    }
    if let Some(rest) = s.strip_prefix("1337;SetUserVar=") {
        let (name, value) = rest.split_once('=')?;
        // `RiftPath` (the shell's $PATH, for command completion) is longer
        // than the others; still bounded by MAX_OSC, and never clipped (a
        // cut $PATH would name a directory that is not on it)
        let is_path = name == USER_VAR_PATH;
        let max_b64 = if is_path { MAX_OSC } else { 1024 };
        if name.is_empty() || name.len() > 64 || value.len() > max_b64 {
            return None;
        }
        let value = String::from_utf8(base64_decode(value)?).ok()?;
        let value = if is_path {
            value.chars().filter(|c| !c.is_control()).collect()
        } else {
            clip(&value)
        };
        return Some(Marker::UserVar {
            name: clip(name),
            value,
        });
    }
    if let Some(rest) = s.strip_prefix("7;") {
        // file://host/path — split off scheme + host, percent-decode the path
        let (host, path) = match rest.strip_prefix("file://") {
            Some(r) => match r.find('/') {
                Some(i) => (&r[..i], &r[i..]),
                None => ("", r),
            },
            None => ("", rest),
        };
        let mut path = percent_decode(path);
        // a Windows shell's file://host/C:/Users/… names C:/Users/…
        if let [b'/', d, b':', rest @ ..] = path.as_bytes()
            && d.is_ascii_alphabetic()
            && matches!(rest.first(), None | Some(b'/' | b'\\'))
        {
            path.remove(0);
        }
        return Some(Marker::Cwd {
            path,
            host: clip(&percent_decode(host)),
        });
    }
    None
}

/// Standard base64 (padding optional); `None` on any other byte.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for b in s.bytes().filter(|&b| b != b'=') {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // two hex digits only (`from_str_radix` would also take a sign)
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1..i + 3].iter().all(u8::is_ascii_hexdigit)
            && let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether an OSC 7 host names this machine (`local` is its hostname).
/// Empty and `localhost` count as local; otherwise the first label is
/// compared case-insensitively, since shells report `$HOST` with or without
/// the domain (`mac` vs `mac.local`).
pub fn host_is_local(host: &str, local: &str) -> bool {
    let first = |h: &str| h.split('.').next().unwrap_or("").to_ascii_lowercase();
    host.is_empty() || host.eq_ignore_ascii_case("localhost") || first(host) == first(local)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers(scanner: &mut OscScanner, chunk: &[u8]) -> Vec<Marker> {
        scanner
            .scan(chunk)
            .into_iter()
            .filter_map(|s| s.marker)
            .collect()
    }

    #[test]
    fn detects_bel_terminated() {
        let mut s = OscScanner::new();
        assert_eq!(
            markers(&mut s, b"\x1b]133;A\x07hello"),
            vec![Marker::PromptStart]
        );
    }

    #[test]
    fn user_vars_decode_base64_and_drop_bad_ones() {
        let mut s = OscScanner::new();
        // "my-venv" and an empty value (the variable was unset)
        assert_eq!(
            markers(
                &mut s,
                b"\x1b]1337;SetUserVar=RiftPython=bXktdmVudg==\x07\x1b]1337;SetUserVar=RiftPhpBin=\x07"
            ),
            vec![
                Marker::UserVar {
                    name: "RiftPython".into(),
                    value: "my-venv".into()
                },
                Marker::UserVar {
                    name: "RiftPhpBin".into(),
                    value: String::new()
                },
            ]
        );
        // not base64; control characters are dropped ("a\x1bb")
        assert!(markers(&mut s, b"\x1b]1337;SetUserVar=X=!!\x07").is_empty());
        assert_eq!(
            markers(&mut s, b"\x1b]1337;SetUserVar=X=YRti\x07"),
            vec![Marker::UserVar {
                name: "X".into(),
                value: "ab".into()
            }]
        );
    }

    #[test]
    fn the_path_user_var_is_kept_whole_and_still_bounded() {
        fn b64(data: &[u8]) -> String {
            const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for c in data.chunks(3) {
                let n = (c[0] as u32) << 16
                    | (*c.get(1).unwrap_or(&0) as u32) << 8
                    | *c.get(2).unwrap_or(&0) as u32;
                for i in 0..=c.len() {
                    out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
                }
            }
            out
        }
        let path: String = (0..150).map(|i| format!("/opt/tool{i:03}/bin:")).collect();
        assert!(path.len() > 2000);
        let osc =
            |name: &str, v: &str| format!("\x1b]1337;SetUserVar={name}={}\x07", b64(v.as_bytes()));
        let mut s = OscScanner::new();
        let got = markers(&mut s, osc(USER_VAR_PATH, &path).as_bytes());
        assert_eq!(
            got,
            vec![Marker::UserVar {
                name: USER_VAR_PATH.into(),
                value: path.clone()
            }]
        );
        // any other variable keeps the short bound
        assert!(markers(&mut s, osc("X", &path).as_bytes()).is_empty());
        // over the OSC bound: dropped, not cut
        let long = path.repeat(2);
        assert!(markers(&mut s, osc(USER_VAR_PATH, &long).as_bytes()).is_empty());
    }

    #[test]
    fn shell_widget_messages() {
        let mut s = OscScanner::new();
        assert_eq!(
            markers(
                &mut s,
                b"\x1b]6973;K;rt\x07\x1b]6973;L\x1b\\\x1b]6973;W;picked%20cmd;\x07"
            ),
            vec![
                Marker::ShellWidget(WidgetMsg::Keys("rt".into())),
                Marker::ShellWidget(WidgetMsg::Loaded),
                Marker::ShellWidget(WidgetMsg::Line {
                    before: "picked cmd".into(),
                    after: String::new()
                }),
            ]
        );
        // a command line longer than the plain-OSC cap is kept whole
        let long = "a%20".repeat(2000);
        let osc = format!("\x1b]6973;W;{long};\x07");
        assert_eq!(
            markers(&mut s, osc.as_bytes()),
            vec![Marker::ShellWidget(WidgetMsg::Line {
                before: "a ".repeat(2000),
                after: String::new()
            })]
        );
        // over its own bound: not cut, but the hand-off still ends
        let osc = format!("\x1b]6973;W;{};\x07", "a".repeat(MAX_WIDGET_LINE));
        assert_eq!(
            markers(&mut s, osc.as_bytes()),
            vec![Marker::ShellWidget(WidgetMsg::Dropped)]
        );
        // other verbs keep the plain bound
        let osc = format!("\x1b]6973;K;{}\x07", "r".repeat(MAX_OSC));
        assert!(markers(&mut s, osc.as_bytes()).is_empty());
    }

    #[test]
    fn detects_st_terminated_and_exit_code() {
        let mut s = OscScanner::new();
        assert_eq!(
            markers(&mut s, b"out\x1b]133;D;42\x1b\\"),
            vec![Marker::Done { exit: Some(42) }]
        );
    }

    #[test]
    fn split_across_chunks() {
        let mut s = OscScanner::new();
        assert!(markers(&mut s, b"\x1b]133;").is_empty());
        assert_eq!(
            markers(&mut s, b"C\x07"),
            vec![Marker::OutputStart { command: None }]
        );
    }

    #[test]
    fn segment_offsets() {
        let mut s = OscScanner::new();
        let segs = s.scan(b"ab\x1b]133;A\x07cd");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].end, 10); // through the BEL
        assert_eq!(segs[0].marker, Some(Marker::PromptStart));
        assert_eq!(segs[1].end, 12);
        assert_eq!(segs[1].marker, None);
    }

    #[test]
    fn cwd_host_locality() {
        assert!(host_is_local("", "mac"));
        assert!(host_is_local("localhost", "mac"));
        assert!(host_is_local("Mac.local", "mac"));
        assert!(host_is_local("mac", "MAC.lan"));
        assert!(!host_is_local("build-01", "mac"));
        assert!(!host_is_local("macbox.local", "mac"));
    }

    #[test]
    fn cwd_without_a_host() {
        let mut s = OscScanner::new();
        assert_eq!(
            markers(&mut s, b"\x1b]7;/srv/app\x07"),
            vec![Marker::Cwd {
                path: "/srv/app".into(),
                host: String::new()
            }]
        );
    }

    #[test]
    fn cwd_decoding() {
        let mut s = OscScanner::new();
        assert_eq!(
            markers(&mut s, b"\x1b]7;file://mac.local/Users/maya/my%20dir\x07"),
            vec![Marker::Cwd {
                path: "/Users/maya/my dir".into(),
                host: "mac.local".into()
            }]
        );
    }

    #[test]
    fn a_windows_drive_path_loses_the_url_slash() {
        let mut s = OscScanner::new();
        for (url, want) in [
            ("file://PC/C:/Users/maya/my%20dir", "C:/Users/maya/my dir"),
            ("file://PC/d:", "d:"),
            ("file://PC/C:%5CUsers", "C:\\Users"),
            // not a drive: kept as it came
            ("file://h/CD:/x", "/CD:/x"),
            ("file://h/C:x", "/C:x"),
        ] {
            let input = format!("\x1b]7;{url}\x07");
            let got = markers(&mut s, input.as_bytes());
            assert!(
                matches!(&got[..], [Marker::Cwd { path, .. }] if path == want),
                "{url}: {got:?}"
            );
        }
    }

    #[test]
    fn graphics_and_inline_images() {
        let mut s = OscScanner::new();
        assert_eq!(
            markers(&mut s, b"\x1b_Ga=q,i=1;AAAA\x1b\\x"),
            vec![Marker::Graphics(b"a=q,i=1;AAAA".to_vec())]
        );
        // other APC strings are skipped, split chunks still assemble
        assert!(markers(&mut s, b"\x1b_Xjunk\x1b\\\x1b_Ga=T;").is_empty());
        assert_eq!(
            markers(&mut s, b"QUJD\x1b\\"),
            vec![Marker::Graphics(b"a=T;QUJD".to_vec())]
        );
        // an inline image larger than the plain-OSC cap is kept whole
        let mut osc = b"\x1b]1337;File=inline=1:".to_vec();
        osc.extend(std::iter::repeat_n(b'A', 10_000));
        osc.push(0x07);
        let m = markers(&mut s, &osc);
        assert!(matches!(&m[..], [Marker::InlineImage(b)] if b.len() == 9 + 10_000));
    }

    #[test]
    fn notifications() {
        let mut s = OscScanner::new();
        let n = |t: &str, b: &str| Marker::Notify {
            title: t.into(),
            body: b.into(),
        };
        assert_eq!(
            markers(
                &mut s,
                b"\x1b]9;build done\x07\x1b]777;notify;Tests;42 passed\x1b\\\x1b]99;i=1:p=body;hi\x07\x1b]99;;Title\x07"
            ),
            vec![
                n("build done", ""),
                n("Tests", "42 passed"),
                n("", "hi"),
                n("Title", "")
            ]
        );
        // ConEmu progress is no notification; empty payloads are nothing
        assert_eq!(
            markers(&mut s, b"\x1b]9;4;1;50\x07\x1b]99;i=1;\x07"),
            vec![Marker::Progress(Some(Progress {
                state: ProgressState::Normal,
                pct: Some(50)
            }))]
        );
    }

    #[test]
    fn progress_states_and_terminators() {
        let mut s = OscScanner::new();
        let p = |state, pct| Marker::Progress(Some(Progress { state, pct }));
        use ProgressState::*;
        assert_eq!(
            markers(
                &mut s,
                b"\x1b]9;4;1;42\x1b\\\x1b]9;4;2\x07\x1b]9;4;2;70\x07\x1b]9;4;3;55\x1b\\\x1b]9;4;4;10\x07\x1b]9;4;0;0\x07\x1b]9;4;0\x07\x1b]9;4;1\x07\x1b]9;4;1;\x07"
            ),
            vec![
                p(Normal, Some(42)),
                p(Error, None),
                p(Error, Some(70)),
                // indeterminate takes no value
                p(Indeterminate, None),
                p(Paused, Some(10)),
                Marker::Progress(None),
                Marker::Progress(None),
                p(Normal, Some(0)),
                p(Normal, Some(0)),
            ]
        );
        // a value past 100 is clamped
        assert_eq!(
            markers(&mut s, b"\x1b]9;4;1;150\x07"),
            vec![p(Normal, Some(100))]
        );
    }

    #[test]
    fn malformed_progress_is_ignored() {
        let mut s = OscScanner::new();
        for bad in [
            &b"\x1b]9;4;9;999\x07"[..],
            b"\x1b]9;4;1;abc\x07",
            b"\x1b]9;4;1;-5\x07",
            b"\x1b]9;4;1;+5\x07",
            b"\x1b]9;4;1;50;x\x07",
            b"\x1b]9;4;\x07",
            b"\x1b]9;4;x\x07",
            // past u32: dropped, not clamped
            b"\x1b]9;4;1;99999999999\x07",
        ] {
            assert!(markers(&mut s, bad).is_empty(), "{bad:?}");
        }
        // 64 KiB of digits outgrows the OSC bound: dropped, and the scanner
        // reports the next sequence again
        let mut huge = b"\x1b]9;4;1;".to_vec();
        huge.extend(std::iter::repeat_n(b'9', 64 << 10));
        huge.extend(b"\x1b\\\x1b]9;4;3\x07");
        assert_eq!(
            markers(&mut s, &huge),
            vec![Marker::Progress(Some(Progress {
                state: ProgressState::Indeterminate,
                pct: None
            }))]
        );
    }

    #[test]
    fn esc_inside_unterminated_string_starts_the_next_sequence() {
        let mut s = OscScanner::new();
        // unterminated OSC, then a real marker: the marker must not be lost
        assert_eq!(
            markers(&mut s, b"\x1b]0;title\x1b]133;A\x07"),
            vec![Marker::PromptStart]
        );
        // unterminated APC, then a marker
        assert_eq!(
            markers(&mut s, b"\x1b_Xjunk\x1b]133;B\x07"),
            vec![Marker::CommandStart]
        );
        // unterminated OSC, then a kitty graphics command
        assert_eq!(
            markers(&mut s, b"\x1b]2;t\x1b_Ga=q;\x1b\\"),
            vec![Marker::Graphics(b"a=q;".to_vec())]
        );
        // a stray G inside a non-kitty APC is data, not a graphics command
        assert!(markers(&mut s, b"\x1b_XabcGdef\x1b\\").is_empty());
        assert!(markers(&mut s, b"\x1b_Xa\x1b\\").is_empty());
    }

    #[test]
    fn can_and_sub_abort_strings() {
        let mut s = OscScanner::new();
        for abort in [0x18u8, 0x1a] {
            let mut input = b"\x1b]133;A".to_vec();
            input.push(abort);
            input.extend_from_slice(b"\x07text");
            assert!(markers(&mut s, &input).is_empty());
            // the scanner is back in ground state
            assert_eq!(
                markers(&mut s, b"\x1b]133;C\x07"),
                vec![Marker::OutputStart { command: None }]
            );
        }
        let mut input = b"\x1b_Ga=T;".to_vec();
        input.push(0x18);
        input.extend_from_slice(b"AAAA\x1b\\");
        assert!(markers(&mut s, &input).is_empty());
    }

    #[test]
    fn output_start_carries_the_command_line() {
        let mut s = OscScanner::new();
        let cmd = |input: &[u8]| match markers(&mut OscScanner::new(), input).as_slice() {
            [Marker::OutputStart { command }] => command.clone(),
            other => panic!("{other:?}"),
        };
        // percent-encoded: the separator, the terminators, newlines, UTF-8
        assert_eq!(
            cmd(b"\x1b]133;C;cmdline_url=echo%20a%3Bb%0Aecho%20%E6%BC%A2%07%25\x07").as_deref(),
            Some("echo a;b\necho 漢%")
        );
        // raw UTF-8 and other parameters around it
        assert_eq!(
            cmd("\x1b]133;C;aid=1;cmdline_url=%20ls 漢字\x1b\\".as_bytes()).as_deref(),
            Some("ls 漢字")
        );
        // a byte that is not UTF-8 neither drops the marker nor the line
        assert_eq!(
            cmd(b"\x1b]133;C;cmdline_url=ls \xff\x07").as_deref(),
            Some("ls \u{fffd}")
        );
        // escapes never reach the header or a re-run
        assert_eq!(
            cmd(b"\x1b]133;C;cmdline_url=a%1B[31mb\x07").as_deref(),
            Some("a[31mb")
        );
        assert_eq!(cmd(b"\x1b]133;C;cmdline_url=\x07"), None);
        // longer than any other OSC string, up to its own bound
        let long = "x".repeat(MAX_OSC * 2);
        let input = format!("\x1b]133;C;cmdline_url={long}\x07");
        assert_eq!(cmd(input.as_bytes()), Some(long));
        // past it the command still starts, without its text
        let mut input = b"\x1b]133;C;cmdline_url=".to_vec();
        input.extend(std::iter::repeat_n(b'x', MAX_CMDLINE));
        input.push(0x07);
        assert_eq!(cmd(&input), None);
        // a clipped string of another kind stays dropped
        let mut input = b"\x1b]133;Cx".to_vec();
        input.extend(std::iter::repeat_n(b'x', MAX_CMDLINE));
        input.push(0x07);
        assert!(markers(&mut s, &input).is_empty());
    }

    #[test]
    fn an_oversized_osc_is_dropped_not_cut() {
        let mut s = OscScanner::new();
        let mut input = b"\x1b]7;file://h/".to_vec();
        input.extend(std::iter::repeat_n(b'a', MAX_OSC));
        input.push(0x07);
        assert!(markers(&mut s, &input).is_empty());
        // the next sequence is reported again
        assert_eq!(
            markers(&mut s, b"\x1b]7;file://h/ok\x1b\\"),
            vec![Marker::Cwd {
                path: "/ok".into(),
                host: "h".into()
            }]
        );
    }

    #[test]
    fn percent_escapes_need_two_hex_digits() {
        let mut s = OscScanner::new();
        assert_eq!(
            markers(&mut s, b"\x1b]7;file://h/a%+1b%2x%41\x07"),
            vec![Marker::Cwd {
                path: "/a%+1b%2xA".into(),
                host: "h".into()
            }]
        );
    }

    #[test]
    fn ignores_other_osc_and_csi() {
        let mut s = OscScanner::new();
        assert!(markers(&mut s, b"\x1b]0;title\x07\x1b[31mred\x1b[0m").is_empty());
    }
}
