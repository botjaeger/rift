/// Streaming scanner for the OSC sequences Shift cares about:
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
    /// OSC 133;D;<exit> — command finished.
    Done { exit: Option<i32> },
    /// OSC 7;file://host/path — working directory changed.
    Cwd(String),
    /// APC `G…` — kitty graphics command (the bytes after `G`).
    Graphics(Vec<u8>),
    /// OSC 1337;File=… — iTerm2 inline image (the bytes after `File=`).
    InlineImage(Vec<u8>),
    /// A desktop-notification request: OSC 9 (iTerm2/ConEmu), OSC 777
    /// `notify` (urxvt/VTE) or OSC 99 (kitty).
    Notify { title: String, body: String },
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Ground,
    Esc,
    Osc,
    /// ESC seen inside an OSC string (possible ST terminator).
    OscEsc,
    /// APC string (`ESC _ … ESC \`).
    Apc,
    ApcEsc,
}

const MAX_OSC: usize = 4096;
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
}

impl OscScanner {
    pub fn new() -> Self {
        Self::default()
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
                State::Esc => {
                    self.buf.clear();
                    self.state = match b {
                        b']' => State::Osc,
                        b'_' => State::Apc,
                        _ => State::Ground,
                    };
                }
                State::Apc => {
                    if b == 0x1b {
                        self.state = State::ApcEsc;
                    } else if self.buf.len() < MAX_IMAGE && (!self.buf.is_empty() || b == b'G') {
                        // only kitty graphics (`G`) bodies are kept
                        self.buf.push(b);
                    }
                }
                State::ApcEsc => {
                    self.state = State::Ground;
                    if b == b'\\' && self.buf.first() == Some(&b'G') && self.buf.len() < MAX_IMAGE {
                        segments.push(Segment {
                            end: i + 1,
                            marker: Some(Marker::Graphics(self.buf[1..].to_vec())),
                        });
                    }
                    self.buf.clear();
                }
                State::Osc => {
                    if b == 0x07 {
                        self.state = State::Ground;
                        if let Some(marker) = parse_osc(&self.buf) {
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
                    }
                }
                State::OscEsc => {
                    self.state = State::Ground;
                    if b == b'\\'
                        && let Some(marker) = parse_osc(&self.buf)
                    {
                        segments.push(Segment {
                            end: i + 1,
                            marker: Some(marker),
                        });
                    }
                    self.buf.clear();
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
    if let Some(rest) = s.strip_prefix("7;") {
        // file://host/path — strip scheme + host, percent-decode the path
        let path = rest
            .strip_prefix("file://")
            .map_or(rest, |r| r.find('/').map(|i| &r[i..]).unwrap_or(r));
        return Some(Marker::Cwd(percent_decode(path)));
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
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
    fn cwd_decoding() {
        let mut s = OscScanner::new();
        assert_eq!(
            markers(&mut s, b"\x1b]7;file://mac.local/Users/maya/my%20dir\x07"),
            vec![Marker::Cwd("/Users/maya/my dir".into())]
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
    fn ignores_other_osc_and_csi() {
        let mut s = OscScanner::new();
        assert!(markers(&mut s, b"\x1b]0;title\x07\x1b[31mred\x1b[0m").is_empty());
    }
}
