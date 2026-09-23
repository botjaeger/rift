/// Streaming scanner for the OSC sequences Runway cares about:
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
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Ground,
    Esc,
    Osc,
    /// ESC seen inside an OSC string (possible ST terminator).
    OscEsc,
}

const MAX_OSC: usize = 4096;

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
                    self.state = if b == b']' {
                        self.buf.clear();
                        State::Osc
                    } else {
                        State::Ground
                    };
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
                    } else if self.buf.len() < MAX_OSC {
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
    fn ignores_other_osc_and_csi() {
        let mut s = OscScanner::new();
        assert!(markers(&mut s, b"\x1b]0;title\x07\x1b[31mred\x1b[0m").is_empty());
    }
}
