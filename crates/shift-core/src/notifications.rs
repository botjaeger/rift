//! Notification preferences (`[notifications]` in config.toml), sound
//! lookup, and recognizing coding agents (Claude Code, Codex, …) and what
//! their terminal notifications mean.
//!
//! Every alert-worthy event lands in the notifications inbox regardless of
//! these switches; the switches decide whether it also raises an in-window
//! toast (Shift focused, pane out of view), a desktop notification (Shift
//! in the background) and a sound.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `[notifications]` table (Warp's Settings → Notifications).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    /// Master switch for toasts, desktop notifications and sounds.
    pub enabled: bool,
    /// A command failed in a pane out of view.
    pub command_failed: bool,
    /// A coding agent (Claude Code, Codex, …) finished its task.
    pub agent_task_completed: bool,
    /// A program asked for attention: OSC 9/777/99, an agent waiting for
    /// input or approval.
    pub needs_attention: bool,
    /// A running command shows a password prompt.
    pub password_prompt: bool,
    /// In-window toasts while Shift is focused.
    pub toasts: bool,
    /// Seconds a toast stays (hovering keeps it).
    pub toast_secs: u64,
    pub play_sound: bool,
    /// `default`, `none`, a sound name (`Glass`, `Hero`, a file stem in
    /// `~/Library/Sounds`) or a path to an audio file.
    pub sound: String,
    /// Per-event overrides; empty = `sound`.
    pub sound_finished: String,
    pub sound_failed: String,
    pub sound_attention: String,
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications {
            enabled: true,
            command_failed: true,
            agent_task_completed: true,
            needs_attention: true,
            password_prompt: true,
            toasts: true,
            toast_secs: 8,
            play_sound: true,
            sound: "default".into(),
            sound_finished: String::new(),
            sound_failed: String::new(),
            sound_attention: String::new(),
        }
    }
}

/// What an alert reports (sound and switch selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A long command or an agent's task finished.
    Finished,
    Failed,
    /// Needs you: a program notification, an agent waiting, a password.
    Attention,
}

impl Notifications {
    /// The configured sound value for `event` (override, else the default).
    pub fn sound_setting(&self, event: Event) -> &str {
        let specific = match event {
            Event::Finished => &self.sound_finished,
            Event::Failed => &self.sound_failed,
            Event::Attention => &self.sound_attention,
        };
        if specific.trim().is_empty() {
            &self.sound
        } else {
            specific
        }
    }

    /// Seconds a toast stays, clamped to 2–60.
    pub fn toast_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.toast_secs.clamp(2, 60))
    }
}

/// Directories searched for named sounds, most specific first.
pub fn sound_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join("Library/Sounds"));
        dirs.push(home.join(".local/share/sounds"));
    }
    dirs.push(PathBuf::from("/System/Library/Sounds"));
    dirs.push(PathBuf::from("/usr/share/sounds/freedesktop/stereo"));
    dirs
}

const AUDIO_EXT: [&str; 7] = ["aiff", "aif", "wav", "caf", "mp3", "m4a", "oga"];

fn is_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXT.contains(&e.to_ascii_lowercase().as_str()))
}

/// Platform sound for `default`: macOS system sounds, freedesktop on Linux.
fn default_name(event: Event) -> &'static [&'static str] {
    match event {
        Event::Finished => &["Glass", "complete"],
        Event::Failed => &["Basso", "dialog-error"],
        Event::Attention => &["Ping", "message-new-instant"],
    }
}

/// Resolve a sound setting to a file: `none` → None, `default` → the
/// platform sound for `event`, a name → first `<dir>/<name>.<ext>`, a path
/// (`/…`, `~/…`) → that file if it exists.
pub fn resolve_sound(value: &str, event: Event, dirs: &[PathBuf]) -> Option<PathBuf> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return None;
    }
    if v.contains('/') {
        let path = match (v.strip_prefix("~/"), std::env::var_os("HOME")) {
            (Some(rest), Some(h)) => PathBuf::from(h).join(rest),
            _ => PathBuf::from(v),
        };
        return path.is_file().then_some(path);
    }
    let names: Vec<&str> = if v.is_empty() || v.eq_ignore_ascii_case("default") {
        default_name(event).to_vec()
    } else {
        vec![v]
    };
    for name in names {
        for dir in dirs {
            for ext in AUDIO_EXT {
                let p = dir.join(format!("{name}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Sound names offered in Settings: `default`, `none`, then every audio
/// file stem in `dirs` (deduplicated, in directory order, sorted within).
pub fn available_sounds(dirs: &[PathBuf]) -> Vec<String> {
    let mut out = vec!["default".to_string(), "none".to_string()];
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut names: Vec<String> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_audio(p))
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        for n in names {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
                out.push(n);
            }
        }
    }
    out
}

/// A coding agent running in a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Codex,
    OpenCode,
    Gemini,
}

impl Agent {
    pub fn name(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
            Agent::OpenCode => "OpenCode",
            Agent::Gemini => "Gemini CLI",
        }
    }

    /// Agent behind a command line: the program word of any of its
    /// `;` / `&&` / `||` / `|` parts (quotes and backslashes respected),
    /// after `VAR=value` assignments, `sudo`/`nohup`/`env`/`exec`/…,
    /// `npx`/`bunx`/`pnpm dlx` and their flags, and any directory prefix.
    pub fn from_command(command: &str) -> Option<Agent> {
        split_commands(command)
            .iter()
            .find_map(|words| Self::from_simple_command(words))
    }

    fn from_simple_command(words: &[String]) -> Option<Agent> {
        let mut words = words.iter().map(String::as_str).peekable();
        // after a prefix command, its `-flags` precede the program
        let mut skip_flags = false;
        let program = loop {
            let w = words.next()?;
            if is_assignment(w) || (skip_flags && w.starts_with('-')) {
                continue;
            }
            if matches!(w, "pnpm" | "yarn") && words.peek() == Some(&"dlx") {
                words.next();
                skip_flags = true;
                continue;
            }
            // `command -v claude` looks the program up; it does not run it
            if w == "command"
                && words
                    .peek()
                    .is_some_and(|n| n.starts_with('-') && n.contains(['v', 'V']))
            {
                return None;
            }
            if matches!(
                w,
                "sudo" | "nohup" | "env" | "npx" | "bunx" | "pnpx" | "exec" | "command" | "time"
            ) {
                skip_flags = true;
                continue;
            }
            break w;
        };
        let base = program.rsplit('/').next().unwrap_or(program);
        let base = base.split('@').next().unwrap_or(base);
        match base {
            "claude" | "claude-code" => Some(Agent::ClaudeCode),
            "codex" => Some(Agent::Codex),
            "opencode" => Some(Agent::OpenCode),
            "gemini" => Some(Agent::Gemini),
            _ => None,
        }
    }
}

/// `NAME=value` (shell variable assignment).
fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Split a command line into simple commands at unquoted `;`, `&`, `|`
/// and newlines, each a list of words with quotes and backslash escapes
/// removed. Empty commands are dropped.
fn split_commands(line: &str) -> Vec<Vec<String>> {
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => word.push(c),
            (Some('"'), '"') => quote = None,
            (Some('"'), '\\') => match chars.next() {
                Some(n @ ('"' | '\\' | '$' | '`')) => word.push(n),
                Some(n) => {
                    word.push('\\');
                    word.push(n);
                }
                None => word.push('\\'),
            },
            (Some(_), _) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    word.push(n);
                }
                in_word = true;
            }
            (None, ';' | '&' | '|' | '\n') | (None, ' ' | '\t') => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                if !matches!(c, ' ' | '\t') && !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
            }
            (None, _) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    if !words.is_empty() {
        commands.push(words);
    }
    commands
}

/// What a terminal notification's text means (Warp's Complete / Request /
/// Error), from the wording agents use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meaning {
    Complete,
    Request,
    Error,
    Info,
}

/// Phrases are matched as whole words ("incomplete" is not "complete").
/// Order: an explicit request phrase, then an error word (unless negated:
/// "no errors", "0 errors", "without error"), then a bare request word
/// ("Grant permission?"), then a completion word.
pub fn classify(text: &str) -> Meaning {
    let t = text.to_lowercase();
    let has = |phrases: &[&str]| phrases.iter().any(|p| phrase_at(&t, p).next().is_some());
    let has_error = [
        "permission denied",
        "access denied",
        "denied",
        "failed",
        "failure",
        "failures",
        "error",
        "errors",
        "errored",
    ]
    .iter()
    .any(|p| phrase_at(&t, p).any(|at| !negated(&t[..at])));
    if has(&[
        "approval requested",
        "needs your permission",
        "requires your approval",
        "waiting for your input",
        "waiting for input",
        "input required",
        "needs your",
        "requires your",
        "please confirm",
        "confirm?",
        "confirmation required",
    ]) {
        Meaning::Request
    } else if has_error {
        Meaning::Error
    } else if has(&["permission", "approval", "approve"]) {
        Meaning::Request
    } else if has(&[
        "complete",
        "completed",
        "finished",
        "done",
        "turn ended",
        "task ended",
    ]) {
        Meaning::Complete
    } else {
        Meaning::Info
    }
}

/// Byte offsets where `phrase` occurs in `text` as whole words: no
/// alphanumeric character directly before or after it.
fn phrase_at<'a>(text: &'a str, phrase: &'a str) -> impl Iterator<Item = usize> + 'a {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    text.match_indices(phrase).filter_map(move |(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + phrase.len()..].chars().next();
        let edge_ok = |edge: Option<char>, inner: Option<char>| !(word(edge) && word(inner));
        (edge_ok(before, phrase.chars().next()) && edge_ok(after, phrase.chars().next_back()))
            .then_some(at)
    })
}

/// `prefix` (text before an error word) carries a negation within the three
/// words before it, in the same clause: "no", "0", "zero", "without" ("no
/// errors", "no test failures").
fn negated(prefix: &str) -> bool {
    if !prefix.ends_with(char::is_whitespace) {
        return false;
    }
    let clause = prefix
        .rsplit(['.', ',', ';', ':', '!', '?', '(', ')'])
        .next()
        .unwrap_or("");
    clause
        .split_whitespace()
        .rev()
        .take(3)
        .any(|w| matches!(w, "no" | "0" | "zero" | "without"))
}

/// A password prompt at the end of the cursor line (`Password:`,
/// `[sudo] password for me:`, `Enter passphrase for key '…':`).
pub fn looks_like_password_prompt(line: &str) -> bool {
    let t = line.trim_end().to_lowercase();
    (t.ends_with(':') || t.ends_with(": ") || t.ends_with('?'))
        && ["password", "passphrase", "passcode", "pin for"]
            .iter()
            .any(|w| t.contains(w))
        && t.chars().count() < 200
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_table() {
        let n: Notifications = toml::from_str("toast_secs = 3\nsound = \"Hero\"").unwrap();
        assert_eq!(n.toast_secs, 3);
        assert!(n.enabled && n.needs_attention && n.play_sound);
        assert_eq!(n.sound_setting(Event::Failed), "Hero");
        let n = Notifications {
            sound_failed: "Basso".into(),
            ..n
        };
        assert_eq!(n.sound_setting(Event::Failed), "Basso");
        assert_eq!(n.sound_setting(Event::Finished), "Hero");
        assert_eq!(n.toast_duration().as_secs(), 3);
        let n = Notifications { toast_secs: 0, ..n };
        assert_eq!(n.toast_duration().as_secs(), 2);
    }

    #[test]
    fn sound_resolution() {
        let dir = std::env::temp_dir().join(format!("shift-sounds-{}", std::process::id()));
        let user = dir.join("user");
        let sys = dir.join("sys");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(sys.join("Glass.aiff"), b"x").unwrap();
        std::fs::write(sys.join("Basso.aiff"), b"x").unwrap();
        std::fs::write(user.join("Chime.wav"), b"x").unwrap();
        std::fs::write(user.join("notes.txt"), b"x").unwrap();
        let dirs = vec![user.clone(), sys.clone()];

        assert_eq!(resolve_sound("none", Event::Finished, &dirs), None);
        assert_eq!(
            resolve_sound("default", Event::Finished, &dirs),
            Some(sys.join("Glass.aiff"))
        );
        assert_eq!(
            resolve_sound("default", Event::Failed, &dirs),
            Some(sys.join("Basso.aiff"))
        );
        // no Ping in the fixture: default for attention finds nothing
        assert_eq!(resolve_sound("default", Event::Attention, &dirs), None);
        assert_eq!(
            resolve_sound("Chime", Event::Attention, &dirs),
            Some(user.join("Chime.wav"))
        );
        assert_eq!(resolve_sound("Missing", Event::Attention, &dirs), None);
        let path = user.join("Chime.wav");
        assert_eq!(
            resolve_sound(&path.to_string_lossy(), Event::Finished, &[]),
            Some(path)
        );
        assert_eq!(
            available_sounds(&dirs),
            vec!["default", "none", "Chime", "Basso", "Glass"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agents_from_commands() {
        assert_eq!(Agent::from_command("claude"), Some(Agent::ClaudeCode));
        assert_eq!(
            Agent::from_command("FOO=1 /opt/homebrew/bin/claude --resume"),
            Some(Agent::ClaudeCode)
        );
        assert_eq!(Agent::from_command("npx codex exec hi"), Some(Agent::Codex));
        assert_eq!(
            Agent::from_command("bunx opencode@latest"),
            Some(Agent::OpenCode)
        );
        assert_eq!(Agent::from_command("gemini"), Some(Agent::Gemini));
        assert_eq!(
            Agent::from_command("cd repo && claude -p 'fix it'"),
            Some(Agent::ClaudeCode)
        );
        assert_eq!(
            Agent::from_command("export PATH=/x:$PATH; codex"),
            Some(Agent::Codex)
        );
        assert_eq!(Agent::from_command("git commit -m claude"), None);
        assert_eq!(Agent::from_command("command -v claude"), None);
        assert_eq!(Agent::from_command("command -V codex"), None);
        assert_eq!(
            Agent::from_command("command claude"),
            Some(Agent::ClaudeCode)
        );
        assert_eq!(Agent::from_command(""), None);
        // separators inside quotes or escaped do not split
        assert_eq!(Agent::from_command("echo \"a; claude go\""), None);
        assert_eq!(Agent::from_command("echo 'a | codex'"), None);
        assert_eq!(Agent::from_command("echo a\\; claude"), None);
        // prefix commands and their flags
        assert_eq!(Agent::from_command("sudo claude"), Some(Agent::ClaudeCode));
        assert_eq!(
            Agent::from_command("nohup env -i FOO=1 gemini"),
            Some(Agent::Gemini)
        );
        assert_eq!(Agent::from_command("npx -y codex"), Some(Agent::Codex));
        assert_eq!(
            Agent::from_command("pnpm dlx --silent opencode"),
            Some(Agent::OpenCode)
        );
        assert_eq!(
            Agent::from_command("\"claude\" -c"),
            Some(Agent::ClaudeCode)
        );
    }

    #[test]
    fn meanings_by_whole_words() {
        let cases = [
            ("Permission denied (publickey)", Meaning::Error),
            ("Access denied", Meaning::Error),
            ("Confirmation complete", Meaning::Complete),
            ("Task completed", Meaning::Complete),
            ("No errors found", Meaning::Info),
            ("Lint passed with 0 errors", Meaning::Info),
            ("Finished without errors", Meaning::Complete),
            ("No errors, 2 failures", Meaning::Error),
            ("Completed, no test failures", Meaning::Complete),
            ("Done with zero lint errors", Meaning::Complete),
            ("Task abandoned", Meaning::Info),
            ("Build incomplete", Meaning::Info),
            ("Terror alert", Meaning::Info),
            ("Please confirm the deploy", Meaning::Request),
            ("Deploy to prod, confirm?", Meaning::Request),
            ("Confirmation required", Meaning::Request),
            ("Requires your approval: rm -rf", Meaning::Request),
        ];
        for (text, want) in cases {
            assert_eq!(classify(text), want, "{text}");
        }
    }

    #[test]
    fn meanings() {
        assert_eq!(
            classify("Claude needs your permission to use Bash"),
            Meaning::Request
        );
        assert_eq!(
            classify("Claude is waiting for your input"),
            Meaning::Request
        );
        assert_eq!(
            classify("Approval requested: rm -rf build"),
            Meaning::Request
        );
        // each keyword on its own, so dropping one fails here
        assert_eq!(classify("Grant permission to edit?"), Meaning::Request);
        assert_eq!(classify("Agent turn complete: fixed it"), Meaning::Complete);
        assert_eq!(classify("Build failed"), Meaning::Error);
        // a request wins over an error word in the same text
        assert_eq!(classify("Approval requested after error"), Meaning::Request);
        assert_eq!(classify("hello"), Meaning::Info);
    }

    #[test]
    fn password_prompts() {
        assert!(looks_like_password_prompt("Password:"));
        assert!(looks_like_password_prompt("[sudo] password for me: "));
        assert!(looks_like_password_prompt(
            "Enter passphrase for key '/Users/me/.ssh/id_ed25519':"
        ));
        assert!(!looks_like_password_prompt("password reset done"));
        assert!(!looks_like_password_prompt("$ echo password"));
    }
}
