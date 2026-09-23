//! Scripted input driver (`shift-terminal --script <file>`).
//!
//! Synthesizes key and mouse events through the app's *real* routing
//! (`App::on_key`, `App::on_left`, …) and captures rendered frames to PNG,
//! so every UI feature can be exercised and verified headlessly and
//! repeatably — no accessibility permissions, no synthetic OS events that
//! could land in another application.
//!
//! One command per line, `#` starts a comment:
//!
//! ```text
//! wait 500                 # sleep before the next step (ms)
//! key cmd+t                # one chord (same syntax as [keybindings])
//! type echo hello          # characters; `\n` submits (Enter)
//! shot /tmp/frame.png      # capture the next rendered frame
//! dump /tmp/state.txt      # write a textual state snapshot
//! resize 900 600           # logical window size
//! move 400 300             # pointer to logical coords
//! click 400 300            # left click (move + press + release)
//! dblclick 400 300         # double click (word selection)
//! rclick 400 300           # right click (block context menu)
//! drag 100 200 400 200     # left-drag between two points
//! scroll 5                 # wheel: +N lines up, −N down
//! sh touch /tmp/x          # run a shell command synchronously
//! focus off                # pretend the window lost/gained focus
//! ime 한글                  # text committed by an input method
//! preedit 하                # IME composition in progress ("" clears)
//! drop /tmp/my file.txt     # a file dropped on the window
//! menu action:new_tab       # choose a menu bar item by id (macOS)
//! mods alt                  # hold modifiers for mouse steps (`mods none`)
//! wait_prompt              # block until the shell shows its first prompt
//! quit                     # exit the app
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::input::KeyPress;

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Wait(u64),
    Key(String),
    Type(String),
    Shot(PathBuf),
    Dump(PathBuf),
    Resize(u32, u32),
    Move(f32, f32),
    Click(f32, f32),
    DoubleClick(f32, f32),
    RightClick(f32, f32),
    Drag(f32, f32, f32, f32),
    Scroll(f32),
    Sh(String),
    Focus(bool),
    Ime(String),
    Preedit(String),
    Drop(PathBuf),
    Menu(String),
    Mods(String),
    WaitPrompt,
    Quit,
}

/// Delay between consecutive input steps so the app (and the shell behind
/// the PTY) can react before the next one lands.
const STEP_GAP: Duration = Duration::from_millis(40);
/// A capture needs a redraw to happen first.
const SHOT_GAP: Duration = Duration::from_millis(200);

pub struct Script {
    steps: Vec<Step>,
    idx: usize,
    due: Instant,
    /// When the current `wait_prompt` started (for its timeout).
    waiting_since: Option<Instant>,
}

impl Script {
    pub fn load(path: &std::path::Path) -> Result<Script, String> {
        let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let steps = parse(&src)?;
        Ok(Script {
            steps,
            idx: 0,
            // let the shell come up before the first step
            due: Instant::now() + Duration::from_millis(1500),
            waiting_since: None,
        })
    }

    pub fn is_done(&self) -> bool {
        self.idx >= self.steps.len()
    }

    /// When the next step should run (None once exhausted).
    pub fn due(&self) -> Option<Instant> {
        (!self.is_done()).then_some(self.due)
    }

    /// Re-run the step that was just taken after `delay`, unless it has
    /// been retrying for longer than `timeout`. Returns true when it gave
    /// up (the script moves on).
    pub fn rearm_wait(&mut self, delay: Duration, timeout: Duration) -> bool {
        let now = Instant::now();
        let since = *self.waiting_since.get_or_insert(now);
        if now.duration_since(since) > timeout {
            log::warn!("script: wait_prompt timed out");
            self.waiting_since = None;
            return true;
        }
        self.idx = self.idx.saturating_sub(1);
        self.due = now + delay;
        false
    }

    /// Pop the next step if it is due.
    pub fn take_due(&mut self, now: Instant) -> Option<Step> {
        if self.is_done() || now < self.due {
            return None;
        }
        let step = self.steps[self.idx].clone();
        self.idx += 1;
        if step != Step::WaitPrompt {
            self.waiting_since = None;
        }
        self.due = now
            + match step {
                Step::Wait(ms) => Duration::from_millis(ms),
                Step::Shot(_) => SHOT_GAP,
                _ => STEP_GAP,
            };
        Some(step)
    }
}

pub fn parse(src: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for (n, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let bad = |what: &str| format!("line {}: {what}: {raw:?}", n + 1);
        let nums = |k: usize| -> Result<Vec<f32>, String> {
            let v: Vec<f32> = rest
                .split_whitespace()
                .map(|s| s.parse::<f32>())
                .collect::<Result<_, _>>()
                .map_err(|_| bad("expected numbers"))?;
            if v.len() != k {
                return Err(bad(&format!("expected {k} numbers")));
            }
            Ok(v)
        };
        let step = match cmd {
            "wait" => Step::Wait(rest.parse().map_err(|_| bad("expected ms"))?),
            "key" => {
                if rest.is_empty() {
                    return Err(bad("expected chord"));
                }
                Step::Key(rest.to_string())
            }
            "type" => Step::Type(unescape(rest)),
            "shot" => Step::Shot(PathBuf::from(rest)),
            "dump" => Step::Dump(PathBuf::from(rest)),
            "resize" => {
                let v = nums(2)?;
                Step::Resize(v[0] as u32, v[1] as u32)
            }
            "move" => {
                let v = nums(2)?;
                Step::Move(v[0], v[1])
            }
            "click" => {
                let v = nums(2)?;
                Step::Click(v[0], v[1])
            }
            "dblclick" => {
                let v = nums(2)?;
                Step::DoubleClick(v[0], v[1])
            }
            "rclick" => {
                let v = nums(2)?;
                Step::RightClick(v[0], v[1])
            }
            "drag" => {
                let v = nums(4)?;
                Step::Drag(v[0], v[1], v[2], v[3])
            }
            "scroll" => Step::Scroll(nums(1)?[0]),
            "sh" => Step::Sh(rest.to_string()),
            "focus" => Step::Focus(matches!(rest, "on" | "true" | "1")),
            "ime" => Step::Ime(unescape(rest)),
            "preedit" => Step::Preedit(unescape(rest)),
            "drop" => Step::Drop(PathBuf::from(rest)),
            "menu" => Step::Menu(rest.to_string()),
            "mods" => Step::Mods(rest.to_string()),
            "wait_prompt" => Step::WaitPrompt,
            "quit" => Step::Quit,
            other => return Err(bad(&format!("unknown command {other:?}"))),
        };
        steps.push(step);
    }
    Ok(steps)
}

/// `\n` → newline (Enter), `\\` → backslash; anything else verbatim.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn named(key: &str) -> Option<NamedKey> {
    Some(match key {
        "tab" => NamedKey::Tab,
        "left" => NamedKey::ArrowLeft,
        "right" => NamedKey::ArrowRight,
        "up" => NamedKey::ArrowUp,
        "down" => NamedKey::ArrowDown,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "enter" | "return" => NamedKey::Enter,
        "escape" | "esc" => NamedKey::Escape,
        "space" => NamedKey::Space,
        "backspace" => NamedKey::Backspace,
        "delete" => NamedKey::Delete,
        "insert" => NamedKey::Insert,
        "f1" => NamedKey::F1,
        "f2" => NamedKey::F2,
        "f3" => NamedKey::F3,
        "f4" => NamedKey::F4,
        "f5" => NamedKey::F5,
        "f6" => NamedKey::F6,
        "f7" => NamedKey::F7,
        "f8" => NamedKey::F8,
        "f9" => NamedKey::F9,
        "f10" => NamedKey::F10,
        "f11" => NamedKey::F11,
        "f12" => NamedKey::F12,
        _ => return None,
    })
}

/// Turn a chord (`cmd+shift+t`, `enter`, `ctrl+c`, `a`) into the key press
/// and modifier state winit would have delivered.
pub fn key_press(chord: &str) -> Option<(KeyPress, ModifiersState)> {
    let b = shift_core::keybindings::Binding::parse(chord)?;
    let mut mods = ModifiersState::empty();
    if b.ctrl {
        mods |= ModifiersState::CONTROL;
    }
    if b.shift {
        mods |= ModifiersState::SHIFT;
    }
    if b.alt {
        mods |= ModifiersState::ALT;
    }
    if b.cmd {
        mods |= ModifiersState::SUPER;
    }
    let press = if let Some(n) = named(&b.key) {
        let text = match n {
            NamedKey::Space => Some(" ".to_string()),
            NamedKey::Enter => Some("\r".to_string()),
            NamedKey::Tab => Some("\t".to_string()),
            _ => None,
        };
        KeyPress {
            logical_key: Key::Named(n),
            text: if b.ctrl || b.cmd { None } else { text },
            unmodified: Some(Key::Named(n)),
        }
    } else {
        let mut c = b.key.chars();
        let ch = c.next()?;
        if c.next().is_some() {
            return None;
        }
        let ch = if b.shift { ch.to_ascii_uppercase() } else { ch };
        let s = ch.to_string();
        KeyPress {
            logical_key: Key::Character(s.clone().into()),
            text: if b.ctrl || b.cmd { None } else { Some(s) },
            unmodified: Some(Key::Character(b.key.clone().into())),
        }
    };
    Some((press, mods))
}

/// A plain typed character (as delivered by the OS for `type` text).
pub fn char_press(ch: char) -> (KeyPress, ModifiersState) {
    let s = ch.to_string();
    let mods = if ch.is_ascii_uppercase() {
        ModifiersState::SHIFT
    } else {
        ModifiersState::empty()
    };
    (
        KeyPress {
            logical_key: Key::Character(s.clone().into()),
            text: Some(s.clone()),
            unmodified: Some(Key::Character(s.to_lowercase().into())),
        },
        mods,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command() {
        let src = "# comment\nwait 10\nkey cmd+t\ntype echo hi\\n\nshot /tmp/a.png\ndump /tmp/s.txt\nresize 800 600\nmove 1 2\nclick 3 4\ndblclick 5 6\nrclick 7 8\ndrag 1 2 3 4\nscroll -3\nsh true\nfocus off\nquit\n";
        let steps = parse(src).unwrap();
        assert_eq!(steps.len(), 15);
        assert_eq!(steps[13], Step::Focus(false));
        assert_eq!(steps[0], Step::Wait(10));
        assert_eq!(steps[2], Step::Type("echo hi\n".into()));
        assert_eq!(steps[10], Step::Drag(1.0, 2.0, 3.0, 4.0));
        assert_eq!(steps[11], Step::Scroll(-3.0));
        assert_eq!(steps[14], Step::Quit);
    }

    #[test]
    fn type_escapes() {
        assert_eq!(unescape("a\\nb"), "a\nb");
        assert_eq!(unescape("printf 'a\\\\nb'"), "printf 'a\\nb'");
        assert_eq!(unescape("x\\q"), "x\\q");
    }

    #[test]
    fn rejects_bad_lines() {
        assert!(parse("bogus 1").is_err());
        assert!(parse("click 1").is_err());
        assert!(parse("wait x").is_err());
    }

    #[test]
    fn chords_become_key_presses() {
        let (p, m) = key_press("cmd+shift+t").unwrap();
        assert_eq!(p.logical_key, Key::Character("T".into()));
        assert_eq!(p.text, None);
        assert!(m.super_key() && m.shift_key());

        let (p, m) = key_press("enter").unwrap();
        assert_eq!(p.logical_key, Key::Named(NamedKey::Enter));
        assert!(m.is_empty());

        let (p, m) = key_press("ctrl+c").unwrap();
        assert_eq!(p.logical_key, Key::Character("c".into()));
        assert_eq!(p.text, None);
        assert!(m.control_key());

        let (p, _) = key_press("a").unwrap();
        assert_eq!(p.text.as_deref(), Some("a"));
        assert!(key_press("").is_none());
    }

    #[test]
    fn step_pacing() {
        let mut s = Script {
            steps: vec![Step::Wait(1000), Step::Quit],
            idx: 0,
            due: Instant::now(),
            waiting_since: None,
        };
        let t0 = Instant::now();
        assert_eq!(s.take_due(t0), Some(Step::Wait(1000)));
        // the wait step pushes the next due time out by its duration
        assert_eq!(s.take_due(t0 + Duration::from_millis(10)), None);
        assert_eq!(
            s.take_due(t0 + Duration::from_millis(1001)),
            Some(Step::Quit)
        );
        assert!(s.is_done());
        assert_eq!(s.due(), None);
    }
}
