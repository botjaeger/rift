//! Rich input (Warp's rich input for CLI agents): Rift's command editor
//! over a program that owns the terminal, a coding agent at its prompt or
//! any full-screen program. The composer docks at the bottom of the pane
//! over the program's screen, which keeps running and drawing above it;
//! sending puts its text into the program as one paste (bracketed when the
//! program enabled bracketed paste) followed by Enter (CR).
//!
//! While it is open the composer's editor is `PaneState::editor` and the
//! pane's own command editor waits in [`Composer::stash`] (the two swap on
//! open and close), so every editing feature (vim mode, multi-cursor, mouse
//! selection, paste, file drop) works in it unchanged.

use std::time::{Duration, Instant};

use rift_core::RichInputSubmit;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::state::EditorState;

/// Quiet after an agent's output that counts as waiting for input
/// (`rich_input_auto`).
pub const AUTO_IDLE: Duration = Duration::from_secs(1);
/// Characters of a command line shown in the header.
const LABEL_MAX: usize = 40;

/// A pane's composer.
pub struct Composer {
    pub open: bool,
    /// The editor not on screen: the pane's command editor while the
    /// composer is open, the composer's draft while it is closed.
    pub stash: EditorState,
    /// What it composes for ("Claude Code", the command line).
    pub label: String,
    /// The running block it composes for; `None`: a program with no block
    /// (a shell without integration). Its end closes the composer.
    pub block: Option<rift_core::BlockId>,
    /// The session's output generation when the composer last closed or
    /// sent: auto-open waits for output after it (an Esc stays closed until
    /// the program says something new).
    pub quiet_gen: u64,
    /// The session's user-input count when the composer last closed or
    /// sent (`None`: never). Output after the user typed into the program
    /// since then (its echo) does not lift the suppression.
    pub quiet_inputs: Option<u64>,
    /// The agent asked for input again (its notification) or worked (its
    /// title) since the last close: auto-open may open regardless of input.
    pub rearmed: bool,
    /// The generation last seen and when it changed (quiet detection).
    pub seen_gen: u64,
    pub changed_at: Instant,
}

impl Default for Composer {
    fn default() -> Self {
        Composer {
            open: false,
            stash: EditorState::default(),
            label: String::new(),
            block: None,
            quiet_gen: 0,
            quiet_inputs: None,
            rearmed: false,
            seen_gen: 0,
            changed_at: Instant::now(),
        }
    }
}

impl Composer {
    /// The draft: the on-screen editor's text while open, else the stash's.
    pub fn draft<'a>(&'a self, on_screen: &'a EditorState) -> &'a str {
        if self.open {
            &on_screen.buffer
        } else {
            &self.stash.buffer
        }
    }

    /// Follow the session's output generation (`gen`), noting when it moved.
    pub fn observe(&mut self, generation: u64, now: Instant) {
        if generation != self.seen_gen {
            self.seen_gen = generation;
            self.changed_at = now;
        }
    }

    /// Opening by itself is allowed: closed, and since the composer last
    /// closed or sent either the agent was re-armed ([`Composer::rearm`]) or
    /// the program produced output while the user typed nothing into it
    /// (`inputs`: the session's user-input count; typing echoes).
    pub fn may_auto_open(&self, inputs: u64) -> bool {
        if self.open {
            return false;
        }
        self.rearmed
            || (self.seen_gen != self.quiet_gen && self.quiet_inputs.is_none_or(|q| q == inputs))
    }

    /// The agent asked for input or worked again: lift an Esc's suppression.
    pub fn rearm(&mut self) {
        self.rearmed = true;
    }

    /// Closed or sent now (`inputs`: the session's user-input count):
    /// auto-open waits for new output the user did not cause, or a re-arm.
    pub fn quiet_now(&mut self, now: Instant, inputs: u64) {
        self.quiet_gen = self.seen_gen;
        self.quiet_inputs = Some(inputs);
        self.rearmed = false;
        self.changed_at = now;
    }
}

/// The header's name for the program: the agent's name, else the command
/// line (first line, cut at [`LABEL_MAX`] characters), else the foreground
/// job.
pub fn label(agent: Option<&str>, command: Option<&str>, job: Option<&str>) -> String {
    if let Some(a) = agent {
        return a.to_string();
    }
    let line = command
        .and_then(|c| c.lines().map(str::trim).find(|l| !l.is_empty()))
        .or_else(|| job.map(str::trim).filter(|j| !j.is_empty()));
    match line {
        Some(l) if l.chars().count() > LABEL_MAX => {
            format!("{}…", l.chars().take(LABEL_MAX - 1).collect::<String>())
        }
        Some(l) => l.to_string(),
        None => "the running program".to_string(),
    }
}

/// The bytes that send `text` to the program: one bracketed paste when it
/// enabled bracketed paste (an embedded end marker cannot end it early),
/// else the text as typed, then CR. `None`: nothing to send (empty or only
/// whitespace).
pub fn send_bytes(text: &str, bracketed: bool) -> Option<Vec<u8>> {
    if text.trim().is_empty() {
        return None;
    }
    let mut bytes = if bracketed {
        crate::input::bracketed_paste(text)
    } else {
        text.as_bytes().to_vec()
    };
    bytes.push(b'\r');
    Some(bytes)
}

/// What a key does in the composer before the editor sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerKey {
    Send,
    Newline,
    Close,
}

/// The composer's own keys: the send key (`rich_input_submit`), a newline
/// on the other Enter, Esc closes. Everything else is the editor's.
pub fn composer_key(
    key: &Key,
    mods: ModifiersState,
    submit: RichInputSubmit,
) -> Option<ComposerKey> {
    match key {
        Key::Named(NamedKey::Escape) if mods.is_empty() => Some(ComposerKey::Close),
        Key::Named(NamedKey::Enter) => {
            if mods.shift_key() || mods.alt_key() || mods.control_key() {
                Some(ComposerKey::Newline)
            } else if mods.super_key() {
                Some(ComposerKey::Send)
            } else {
                Some(match submit {
                    RichInputSubmit::Enter => ComposerKey::Send,
                    RichInputSubmit::CmdEnter => ComposerKey::Newline,
                })
            }
        }
        _ => None,
    }
}

/// The hint row: what sends, what breaks the line, what closes.
pub fn hint(submit: RichInputSubmit) -> &'static str {
    match submit {
        RichInputSubmit::Enter => "⏎ send   ⇧⏎ new line   esc close",
        RichInputSubmit::CmdEnter => "⌘⏎ send   ⏎ new line   esc close",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bracketed_program_gets_one_paste_then_enter() {
        assert_eq!(
            send_bytes("line1\nline2", true).unwrap(),
            b"\x1b[200~line1\nline2\x1b[201~\r".to_vec()
        );
        // an end marker in the text cannot close the paste early
        assert_eq!(
            send_bytes("a\x1b[201~b", true).unwrap(),
            b"\x1b[200~ab\x1b[201~\r".to_vec()
        );
    }

    #[test]
    fn a_program_without_bracketed_paste_gets_the_text_then_enter() {
        assert_eq!(send_bytes("hello", false).unwrap(), b"hello\r".to_vec());
        assert_eq!(send_bytes("a\nb", false).unwrap(), b"a\nb\r".to_vec());
    }

    #[test]
    fn an_empty_composer_sends_nothing() {
        assert_eq!(send_bytes("", true), None);
        assert_eq!(send_bytes("  \n ", false), None);
    }

    #[test]
    fn enter_sends_and_shift_enter_breaks_the_line() {
        let enter = Key::Named(NamedKey::Enter);
        let e = RichInputSubmit::Enter;
        assert_eq!(
            composer_key(&enter, ModifiersState::empty(), e),
            Some(ComposerKey::Send)
        );
        assert_eq!(
            composer_key(&enter, ModifiersState::SHIFT, e),
            Some(ComposerKey::Newline)
        );
        assert_eq!(
            composer_key(&enter, ModifiersState::ALT, e),
            Some(ComposerKey::Newline)
        );
        let c = RichInputSubmit::CmdEnter;
        assert_eq!(
            composer_key(&enter, ModifiersState::empty(), c),
            Some(ComposerKey::Newline)
        );
        assert_eq!(
            composer_key(&enter, ModifiersState::SUPER, c),
            Some(ComposerKey::Send)
        );
        let esc = Key::Named(NamedKey::Escape);
        assert_eq!(
            composer_key(&esc, ModifiersState::empty(), e),
            Some(ComposerKey::Close)
        );
        assert_eq!(
            composer_key(&Key::Character("a".into()), ModifiersState::empty(), e),
            None
        );
    }

    #[test]
    fn auto_open_waits_for_output_after_a_close() {
        let mut c = Composer::default();
        let t = Instant::now();
        assert!(!c.may_auto_open(0), "no output yet");
        c.observe(3, t);
        // the command line typed at the shell counts as input: not yet closed
        assert!(c.may_auto_open(1));
        c.quiet_now(t, 1);
        assert!(!c.may_auto_open(1), "closed: suppressed");
        c.observe(3, t);
        assert!(!c.may_auto_open(1), "same generation: still suppressed");
        c.observe(4, t);
        assert!(c.may_auto_open(1), "new output lifts it");
    }

    #[test]
    fn typing_into_the_program_after_a_close_does_not_reopen_it() {
        let mut c = Composer::default();
        let t = Instant::now();
        c.observe(3, t);
        c.quiet_now(t, 5);
        // the user typed into the agent and it echoed
        c.observe(4, t);
        assert!(!c.may_auto_open(6), "own typing's echo: still suppressed");
        c.observe(9, t);
        assert!(!c.may_auto_open(7), "more output after typing: suppressed");
        // the agent asks again (notification) or worked (title)
        c.rearm();
        assert!(c.may_auto_open(7), "a re-arm lifts it");
        c.quiet_now(t, 7);
        assert!(!c.may_auto_open(7), "the next close clears the re-arm");
    }

    #[test]
    fn the_header_names_the_agent_else_the_command() {
        assert_eq!(
            label(Some("Claude Code"), Some("claude"), None),
            "Claude Code"
        );
        assert_eq!(label(None, Some("  vim notes.md\n"), None), "vim notes.md");
        assert_eq!(label(None, None, Some("less")), "less");
        assert_eq!(label(None, Some(" "), None), "the running program");
        let long = "x".repeat(60);
        assert_eq!(label(None, Some(&long), None).chars().count(), LABEL_MAX);
    }
}
