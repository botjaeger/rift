//! Shell widgets (Warp): ⌃R, ⌃T and ⌥C run the user's own shell widgets
//! (fzf, atuin, fzf.fish) on Rift's command line instead of Rift's own
//! history pane.
//!
//! The hooks (`shell/rift.*`) report at each prompt which of the keys the
//! user's config binds (OSC 6973;K;`<letters r t c>`). For such a key Rift
//! types `%Z<before>%Z<after>` into the shell's own (empty, hidden) line
//! editor — its command line split at the cursor, every byte but ASCII
//! letters and digits as `%XX`, so no character can trigger a binding of
//! its own — then `^X ^Y <letter>`, which the hook binds to a wrapper: it
//! acknowledges (OSC 6973;L), loads the line, runs the user's binding, then
//! reports the line it left (OSC 6973;W;`<before>`;`<after>`,
//! percent-encoded) and empties its own. A binding that runs a command
//! itself (fzf's ⌥C cd) reports nothing in time: the command starts (OSC
//! 133;C) and the next prompt follows. While the hand-off runs, keys go raw
//! to the shell (the widget's TUI draws below the prompt).

/// A key the shell may bind to a widget of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKey {
    /// ⌃R — history search (fzf, atuin).
    History,
    /// ⌃T — file picker (fzf).
    Files,
    /// ⌥C — directory jump (fzf).
    Cd,
}

impl WidgetKey {
    /// The letter the hooks report the key as and its trigger ends with.
    pub fn letter(self) -> char {
        match self {
            WidgetKey::History => 'r',
            WidgetKey::Files => 't',
            WidgetKey::Cd => 'c',
        }
    }
}

/// Longest command line (bytes) handed to the shell: it is typed into the
/// shell's line editor, up to three bytes per byte. A longer one keeps
/// Rift's own behaviour.
pub const MAX_HANDOFF: usize = 4096;

/// A message of the hooks' OSC 6973.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetMsg {
    /// The keys the shell binds to its own widgets, as letters (`rtc`).
    Keys(String),
    /// The wrapper loaded the line: the widget runs.
    Loaded,
    /// The line the widget left, split at its cursor.
    Line { before: String, after: String },
    /// A report too long to keep (the widget's line is lost).
    Dropped,
}

/// Parse an OSC 6973 body (after `6973;`).
pub(crate) fn parse(rest: &str) -> Option<WidgetMsg> {
    let (verb, body) = rest.split_once(';').unwrap_or((rest, ""));
    match verb {
        "K" => {
            let mut keys = String::new();
            for c in body.chars().filter(|c| matches!(c, 'r' | 't' | 'c')) {
                if !keys.contains(c) {
                    keys.push(c);
                }
            }
            Some(WidgetMsg::Keys(keys))
        }
        "L" => Some(WidgetMsg::Loaded),
        "W" => {
            let (before, after) = body.split_once(';')?;
            // a command line: newlines and tabs are the only controls kept
            let clean = |s: &str| -> String {
                crate::osc::percent_decode(s)
                    .chars()
                    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                    .collect()
            };
            Some(WidgetMsg::Line {
                before: clean(before),
                after: clean(after),
            })
        }
        _ => None,
    }
}

/// Every byte but ASCII letters and digits as `%XX`.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The bytes that hand `text` (cursor at byte `cursor`) to the shell's
/// binding for `key`; `None` when the line is too long to hand over.
pub fn handoff_bytes(key: WidgetKey, text: &str, cursor: usize) -> Option<Vec<u8>> {
    if text.len() > MAX_HANDOFF {
        return None;
    }
    let mut at = cursor.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let (before, after) = text.split_at(at);
    let mut out = format!("%Z{}%Z{}", encode(before), encode(after)).into_bytes();
    out.extend_from_slice(&[0x18, 0x19, key.letter() as u8]);
    Some(out)
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Phase {
    #[default]
    Idle,
    /// Rift typed the trigger; the wrapper has not answered yet.
    Sent,
    /// The wrapper loaded the line: the user's widget runs.
    Running,
}

/// A pane's shell-widget state: the keys its shell binds, and a hand-off
/// in flight. Written by the output parser, read by the app.
#[derive(Debug, Default)]
pub struct WidgetState {
    keys: String,
    phase: Phase,
    result: Option<(String, String)>,
    /// A prompt ended the last hand-off before its wrapper answered. The
    /// shell can repaint its prompt on the trigger's bytes before its
    /// wrapper loads (fish does, and under load the repaint wins), so that
    /// wrapper's own `Loaded` still resumes the hand-off.
    unanswered: bool,
}

impl WidgetState {
    /// Whether the shell reported a widget of its own on `key`.
    pub fn bound(&self, key: WidgetKey) -> bool {
        self.keys.contains(key.letter())
    }

    /// A hand-off is in flight: keys go raw to the shell.
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Rift typed the trigger.
    pub fn begin(&mut self) {
        self.phase = Phase::Sent;
        self.result = None;
        self.unanswered = false;
    }

    pub fn on_msg(&mut self, msg: WidgetMsg) {
        match msg {
            WidgetMsg::Keys(keys) => self.keys = keys,
            WidgetMsg::Loaded if self.phase == Phase::Sent || self.unanswered => {
                self.phase = Phase::Running;
                self.unanswered = false;
            }
            WidgetMsg::Loaded => {}
            // only the answer to a hand-off: a stale report (one that
            // comes after its command started) or another program's
            // output never reaches the command line
            WidgetMsg::Line { before, after } if self.busy() => {
                self.phase = Phase::Idle;
                self.result = Some((before, after));
            }
            WidgetMsg::Line { .. } => {}
            WidgetMsg::Dropped => self.phase = Phase::Idle,
        }
    }

    /// OSC 133;A. Before the wrapper answered, a new prompt may mean it
    /// never will (no hook, or ⌃C): the keys go back to the editor, but the
    /// wrapper's own `Loaded` still resumes it (`unanswered`). A widget's
    /// own prompt repaint may emit one while it runs.
    pub fn on_prompt_start(&mut self) {
        if self.phase == Phase::Sent {
            self.phase = Phase::Idle;
            self.unanswered = true;
        }
    }

    /// OSC 133;C: the widget ran a command itself.
    pub fn on_output_start(&mut self) {
        self.phase = Phase::Idle;
        self.unanswered = false;
    }

    /// The line a finished hand-off left, split at its cursor.
    pub fn take_result(&mut self) -> Option<(String, String)> {
        self.result.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_encodes_everything_but_letters_and_digits() {
        let b = handoff_bytes(WidgetKey::History, "ls -l é;%\n", 3).unwrap();
        assert_eq!(
            String::from_utf8(b).unwrap(),
            "%Zls%20%Z%2Dl%20%C3%A9%3B%25%0A\x18\x19r"
        );
        // a cursor inside a character falls back to its start
        let b = handoff_bytes(WidgetKey::Cd, "é", 1).unwrap();
        assert_eq!(String::from_utf8(b).unwrap(), "%Z%Z%C3%A9\x18\x19c");
        assert!(handoff_bytes(WidgetKey::Files, &"x".repeat(MAX_HANDOFF + 1), 0).is_none());
    }

    #[test]
    fn parses_keys_loaded_and_lines() {
        assert_eq!(parse("K;rtc"), Some(WidgetMsg::Keys("rtc".into())));
        // unknown letters and repeats dropped
        assert_eq!(parse("K;rxrq\x07t"), Some(WidgetMsg::Keys("rt".into())));
        assert_eq!(parse("K;"), Some(WidgetMsg::Keys(String::new())));
        assert_eq!(parse("L"), Some(WidgetMsg::Loaded));
        assert_eq!(
            parse("W;git%20log%0A--;%20-p%3B%1B"),
            Some(WidgetMsg::Line {
                before: "git log\n--".into(),
                after: " -p;".into(),
            })
        );
        assert_eq!(parse("W;nosplit"), None);
        assert_eq!(parse("X;1"), None);
    }

    #[test]
    fn phases() {
        let mut s = WidgetState::default();
        s.on_msg(WidgetMsg::Keys("rc".into()));
        assert!(s.bound(WidgetKey::History) && s.bound(WidgetKey::Cd));
        assert!(!s.bound(WidgetKey::Files));
        // a line nobody asked for is ignored
        s.on_msg(WidgetMsg::Line {
            before: "rm -rf".into(),
            after: String::new(),
        });
        assert_eq!(s.take_result(), None);
        // answered
        s.begin();
        assert!(s.busy());
        s.on_msg(WidgetMsg::Loaded);
        s.on_prompt_start(); // a widget's repaint
        assert!(s.busy());
        s.on_msg(WidgetMsg::Line {
            before: "picked".into(),
            after: " cmd".into(),
        });
        assert!(!s.busy());
        assert_eq!(s.take_result(), Some(("picked".into(), " cmd".into())));
        // never answered: the next prompt ends it
        s.begin();
        s.on_prompt_start();
        assert!(!s.busy());
        // the shell repainted its prompt on the trigger before its wrapper
        // loaded (fish, under load): the wrapper's answer still lands
        s.begin();
        s.on_prompt_start();
        s.on_msg(WidgetMsg::Loaded);
        assert!(s.busy());
        s.on_prompt_start(); // the widget's own repaint
        s.on_msg(WidgetMsg::Line {
            before: "aFILE".into(),
            after: " b".into(),
        });
        assert_eq!(s.take_result(), Some(("aFILE".into(), " b".into())));
        // but a stray line after an unanswered hand-off is still dropped
        s.begin();
        s.on_prompt_start();
        s.on_msg(WidgetMsg::Line {
            before: "rm -rf".into(),
            after: String::new(),
        });
        assert_eq!(s.take_result(), None);
        // and a command that started since ends what was left of it
        s.on_output_start();
        s.on_msg(WidgetMsg::Loaded);
        assert!(!s.busy());
        // the widget ran a command: its late report is dropped
        s.begin();
        s.on_msg(WidgetMsg::Loaded);
        s.on_output_start();
        s.on_msg(WidgetMsg::Line {
            before: String::new(),
            after: String::new(),
        });
        assert_eq!(s.take_result(), None);
        // a report too long to keep ends it
        s.begin();
        s.on_msg(WidgetMsg::Dropped);
        assert!(!s.busy());
    }
}
