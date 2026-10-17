/// Streaming scanner for the OSC sequences Rift cares about:
/// OSC 133 (shell-integration block markers) and OSC 7 (cwd report).
/// Bytes are NOT consumed — the same stream is fed to the VT parser; this
/// scanner only spots marker positions so the caller can split the stream
/// and record grid state at each marker boundary.

#[derive(Debug, Clone, PartialEq)]
pub enum Marker {
    /// OSC 133;A — start of prompt.
    PromptStart,
    /// OSC 133;B — end of prompt, user input begins.
    CommandStart,
    /// OSC 133;C — command accepted, output begins.
    OutputStart,
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
/// The user variable carrying the shell's `$PATH` (see `shell/rift.*`).
pub const USER_VAR_PATH: &str = "RiftPath";
/// Inline images and graphics commands carry base64 payloads.
const MAX_IMAGE: usize = 48 << 20;
const ITERM_FILE: &[u8] = b"1337;File=";

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
                        if !std::mem::take(&mut self.clipped)
                            && let Some(marker) = parse_osc(&self.buf)
                        {
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
                    {
                        self.buf.push(b);
                    } else {
                        self.clipped = true;
                    }
                }
                State::OscEsc => {
                    if b == b'\\' {
                        self.state = State::Ground;
                        if !std::mem::take(&mut self.clipped)
                            && let Some(marker) = parse_osc(&self.buf)
                        {
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

fn parse_osc(buf: &[u8]) -> Option<Marker> {
    if let Some(rest) = buf.strip_prefix(ITERM_FILE) {
        return (buf.len() < MAX_IMAGE).then(|| Marker::InlineImage(rest.to_vec()));
    }
    let s = std::str::from_utf8(buf).ok()?;
    if let Some(rest) = s.strip_prefix("133;") {
        let mut parts = rest.split(';');
        return match parts.next()? {
            "A" => Some(Marker::PromptStart),
            "B" => Some(Marker::CommandStart),
            "C" => Some(Marker::OutputStart),
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
        if rest.starts_with("4;") || rest.is_empty() {
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
        return Some(Marker::Cwd {
            path: percent_decode(path),
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

fn percent_decode(s: &str) -> String {
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
        assert_eq!(markers(&mut s, b"C\x07"), vec![Marker::OutputStart]);
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
        // ConEmu progress and empty payloads are not notifications
        assert!(markers(&mut s, b"\x1b]9;4;1;50\x07\x1b]99;i=1;\x07").is_empty());
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
                vec![Marker::OutputStart]
            );
        }
        let mut input = b"\x1b_Ga=T;".to_vec();
        input.push(0x18);
        input.extend_from_slice(b"AAAA\x1b\\");
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
