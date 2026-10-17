//! Completions from the user's own shell: its completion functions, plugins
//! and specs for tools Rift has no spec for (`terraform`, `just`, a user's
//! `_mycmd`).
//!
//! One warm completion shell per shell program ([`Completer`]), started on
//! the first request, reused, and started again after it died or timed out.
//! It runs a small server (`shell/complete.{zsh,bash,fish}`) that reads
//! requests on stdin and answers between markers on stdout:
//!
//! * zsh: the user's interactive zsh in a `zpty`; a widget reads the line as
//!   data, sets the editor buffer to it and runs complete-word (Return is
//!   unbound, the word-evaluating `_expand` completer disabled) and `compadd`
//!   is wrapped to print the matches (zsh-capture-completion's technique);
//! * bash: the completion function the command's `complete -F` spec names
//!   (bash-completion's loader runs first when it is loaded), called with
//!   `COMP_WORDS` set, plus `compgen` for word-list and action specs;
//! * fish: `complete --do-complete=<line>`.
//!
//! The line is data: Rift never sends one holding a control character or a
//! command substitution ([`query`]), and each server passes it to the shell
//! as a variable or as the editor buffer, never as code. Completion
//! functions may run commands, as they do on Tab in the shell itself.
//! Replies are bounded ([`MAX_ITEMS`], [`MAX_FIELD`]) and cleaned of control
//! characters: they come from user code.

use crate::shell_env::ShellKind;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const SERVER_ZSH: &str = include_str!("../../../shell/complete.zsh");
pub const SERVER_BASH: &str = include_str!("../../../shell/complete.bash");
pub const SERVER_FISH: &str = include_str!("../../../shell/complete.fish");

/// Time a request may take, start-up included; a later answer is dropped.
pub const BUDGET: Duration = Duration::from_millis(300);
/// Matches kept from one reply.
pub const MAX_ITEMS: usize = 400;
/// Bytes kept of one word or description; a longer line is cut there.
pub const MAX_FIELD: usize = 512;
/// Longest command line sent.
const MAX_LINE: usize = 2048;
/// Lines of rc output read while waiting for the server to start.
const MAX_STARTUP_LINES: usize = 10_000;
/// A shell that has not started its server by then is stopped.
const START_LIMIT: Duration = Duration::from_secs(10);
/// Pause before starting a shell again after one failed to start or hung.
const COOLDOWN: Duration = Duration::from_secs(5);

/// What one request asks the shell to complete.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Query {
    /// The directory the completion runs in.
    pub cwd: String,
    /// The command line up to the end of the word being completed.
    pub line: String,
    /// The words of the command being completed, unquoted; the last is
    /// the word being completed (possibly empty).
    pub words: Vec<String>,
}

/// One match: the literal word (unquoted; Rift escapes it on insert) and a
/// description.
pub type Item = (String, String);

/// The [`Query`] for completing the word that ends at `end` in `buffer`,
/// or `None` when the shell is not asked: the word is the command name
/// (Rift lists `$PATH`), looks like a path (Rift lists the directory), or
/// the line holds a control character, a command substitution (`$( )`,
/// backticks, `<( )`, `>( )`, `=( )`), for zsh and fish any `(` (zsh glob
/// qualifiers such as `*(e:…:)` and `${(e)…}` flags run code when a
/// completer expands the word) or is too long.
pub fn query(kind: ShellKind, buffer: &str, end: usize, cwd: &str) -> Option<Query> {
    if end > buffer.len() || !buffer.is_char_boundary(end) {
        return None;
    }
    let line = &buffer[..end];
    if line.len() > MAX_LINE
        || line.chars().any(char::is_control)
        || cwd.chars().any(char::is_control)
        || cwd.is_empty()
    {
        return None;
    }
    let substitution = ["$(", "`", "<(", ">(", "=("]
        .iter()
        .any(|s| line.contains(s))
        || (kind != ShellKind::Bash && line.contains('('));
    if substitution {
        return None;
    }
    let (start, token) = crate::completion::current_token(line, end);
    let seg = crate::completion_specs::segment_start(&line[..start]);
    let before = crate::completion_specs::tokenize(&line[seg..start]);
    if before.is_empty() {
        return None;
    }
    let word = crate::completion::unescape_word(token);
    if word.contains('/') || word.starts_with('.') || word.starts_with('~') {
        return None;
    }
    let mut words = before;
    words.push(word);
    if words.iter().any(|w| w.chars().any(char::is_control)) {
        return None;
    }
    Some(Query {
        cwd: cwd.to_string(),
        line: line.to_string(),
        words,
    })
}

/// The request as the servers read it: id, directory, line, the index n of
/// the last word, then the n+1 words, one per line.
pub fn encode(id: u64, q: &Query) -> String {
    let mut s = format!(
        "{id}\n{}\n{}\n{}\n",
        q.cwd,
        q.line,
        q.words.len().saturating_sub(1)
    );
    for w in &q.words {
        s.push_str(w);
        s.push('\n');
    }
    if q.words.is_empty() {
        s.push('\n');
    }
    s
}

/// `s` without ANSI escape sequences and control characters, at most
/// [`MAX_FIELD`] bytes.
pub fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(MAX_FIELD));
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI: ESC [ params final; OSC: ESC ] … BEL or ESC \; other
            // escapes: ESC + one char
            match chars.next() {
                Some('[') => {
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(n) = chars.next() {
                        if n == '\x07' || (n == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if c.is_control() {
            continue;
        }
        if out.len() + c.len_utf8() > MAX_FIELD {
            break;
        }
        out.push(c);
    }
    out
}

/// The matches in reply lines (`\x1f<word>\x1f<description>`): cleaned,
/// empty words skipped, at most [`MAX_ITEMS`], first of each word kept.
pub fn parse_items<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for l in lines {
        let Some(rest) = l.find('\x1f').map(|at| &l[at + 1..]) else {
            continue;
        };
        let (word, desc) = rest.split_once('\x1f').unwrap_or((rest, ""));
        let word = clean(word.trim_end_matches(' '));
        if word.is_empty() || !seen.insert(word.clone()) {
            continue;
        }
        out.push((word, clean(desc.trim())));
        if out.len() >= MAX_ITEMS {
            break;
        }
    }
    out
}

/// Reply lines from a server's stdout, each at most `MAX_FIELD * 2 + 8`
/// bytes (the rest of a longer line is dropped), lossily decoded.
pub fn read_lines(out: impl Read + Send + 'static, tx: Sender<String>) {
    let mut r = BufReader::new(out);
    let cap = MAX_FIELD * 2 + 8;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // read one line, keeping at most `cap` bytes of it
        let mut kept = 0;
        let mut eof = false;
        loop {
            let chunk = match r.fill_buf() {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            if chunk.is_empty() {
                eof = true;
                break;
            }
            let (part, done) = match chunk.iter().position(|&b| b == b'\n') {
                Some(i) => (&chunk[..i], Some(i + 1)),
                None => (chunk, None),
            };
            let room = cap.saturating_sub(kept);
            let take = part.len().min(room);
            buf.extend_from_slice(&part[..take]);
            kept += take;
            let used = done.unwrap_or(chunk.len());
            r.consume(used);
            if done.is_some() {
                break;
            }
        }
        if eof && buf.is_empty() {
            return;
        }
        if tx.send(String::from_utf8_lossy(&buf).into_owned()).is_err() || eof {
            return;
        }
    }
}

/// A running completion server.
struct Running {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    started: Instant,
    ready: bool,
}

impl Running {
    fn stop(mut self) {
        stop_child(&mut self.child);
    }
}

/// Kill the server's process group (the shell and whatever its completion
/// functions started) and reap it off this thread.
fn stop_child(child: &mut Child) {
    let pid = child.id();
    // the group was created for this child (`process_group(0)`)
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// A warm completion shell for one shell program.
pub struct Completer {
    kind: ShellKind,
    program: String,
    env: Vec<(String, String)>,
    running: Option<Running>,
    next_id: u64,
    cooldown_until: Option<Instant>,
}

impl Completer {
    pub fn new(program: &str) -> Completer {
        Completer {
            kind: ShellKind::from_program(program),
            program: program.to_string(),
            env: Vec::new(),
            running: None,
            next_id: 0,
            cooldown_until: None,
        }
    }

    /// With extra environment for the shell (tests: a scratch `HOME`).
    pub fn with_env(mut self, env: &[(&str, &str)]) -> Completer {
        self.env = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self
    }

    pub fn kind(&self) -> ShellKind {
        self.kind
    }

    /// Whether a server process is running (started or starting).
    pub fn is_running(&mut self) -> bool {
        match &mut self.running {
            Some(r) => matches!(r.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// The server's process id, if one runs.
    pub fn pid(&self) -> Option<u32> {
        self.running.as_ref().map(|r| r.child.id())
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        // a login shell on macOS, as the panes and the env probe run
        let login = cfg!(target_os = "macos");
        match self.kind {
            ShellKind::Zsh => {
                cmd.args(["-f", "-c", SERVER_ZSH, "rift-complete", &self.program]);
                if login {
                    cmd.arg("-l");
                }
                // the inner shell's line editor needs a terminal type
                cmd.env("TERM", "xterm-256color");
            }
            ShellKind::Bash => {
                if login {
                    cmd.arg("-l");
                }
                cmd.args(["-i", "-c", SERVER_BASH]);
            }
            ShellKind::Fish => {
                if login {
                    cmd.arg("-l");
                }
                // interactive: config.fish's `status is-interactive` blocks
                // (where completions are often set up) run too
                cmd.args(["-i", "-c", SERVER_FISH]);
            }
        }
        cmd.env("RIFT_COMPLETE_SHELL", "1")
            .env_remove("RIFT_COMPLETE_FPATH");
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd
    }

    fn start(&mut self, now: Instant) -> bool {
        if self.cooldown_until.is_some_and(|t| now < t) {
            return false;
        }
        let mut cmd = self.command();
        {
            use std::os::unix::process::CommandExt;
            // own process group: a timeout stops the functions it ran too
            cmd.process_group(0);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("completion shell {}: {e}", self.program);
                self.cooldown_until = Some(now + COOLDOWN);
                return false;
            }
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            stop_child(&mut child);
            return false;
        };
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("completion-shell".into())
            .spawn(move || read_lines(stdout, tx));
        if spawned.is_err() {
            stop_child(&mut child);
            return false;
        }
        self.running = Some(Running {
            child,
            stdin,
            lines: rx,
            started: now,
            ready: false,
        });
        true
    }

    /// Stop the server (it starts again on the next request).
    pub fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            r.stop();
        }
    }

    fn fail(&mut self, now: Instant, pause: Duration) {
        self.stop();
        self.cooldown_until = Some(now + pause);
    }

    /// The shell's matches for `q`, waiting at most `budget` (start-up
    /// included). Empty when the shell is starting, failed or took longer;
    /// a server that misses the budget mid-request is stopped, so a hung
    /// completion function never delays the next request.
    pub fn complete(&mut self, q: &Query, budget: Duration) -> Vec<Item> {
        let start = Instant::now();
        let deadline = start + budget;
        if !self.is_running() {
            self.stop();
            if !self.start(start) {
                return Vec::new();
            }
        }
        let Some(r) = self.running.as_mut() else {
            return Vec::new();
        };
        self.next_id += 1;
        match exchange(r, self.next_id, q, deadline) {
            Ok(lines) => parse_items(lines.iter().map(String::as_str)),
            Err(Outcome::Starting) => Vec::new(),
            Err(Outcome::Failed) => {
                self.fail(Instant::now(), COOLDOWN);
                Vec::new()
            }
            Err(Outcome::TimedOut) => {
                // a hung completion function: never wait on it again
                log::debug!("completion shell {}: timed out", self.program);
                self.fail(Instant::now(), TIMEOUT_COOLDOWN);
                Vec::new()
            }
        }
    }
}

/// Pause before starting a shell again after a request timed out.
const TIMEOUT_COOLDOWN: Duration = Duration::from_secs(1);

/// Why a request got no reply.
enum Outcome {
    /// Still reading its rc files: keep it starting.
    Starting,
    /// It exited, broke the protocol or never started.
    Failed,
    /// The request missed the deadline.
    TimedOut,
}

/// Send request `id` to `r` and collect its reply lines until `deadline`.
fn exchange(
    r: &mut Running,
    id: u64,
    q: &Query,
    deadline: Instant,
) -> Result<Vec<String>, Outcome> {
    if !r.ready {
        let mut seen = 0;
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            match r.lines.recv_timeout(wait) {
                Ok(l) if l.contains("\x1eready") => {
                    r.ready = true;
                    break;
                }
                Ok(_) => {
                    seen += 1;
                    if seen > MAX_STARTUP_LINES {
                        return Err(Outcome::Failed);
                    }
                }
                Err(RecvTimeoutError::Timeout) if r.started.elapsed() < START_LIMIT => {
                    return Err(Outcome::Starting);
                }
                Err(_) => return Err(Outcome::Failed),
            }
        }
    }
    // anything left over belongs to no request
    while r.lines.try_recv().is_ok() {}
    if r.stdin.write_all(encode(id, q).as_bytes()).is_err() || r.stdin.flush().is_err() {
        return Err(Outcome::Failed);
    }
    let begin = format!("\x1ebegin {id}");
    let end = format!("\x1eend {id}");
    let mut inside = false;
    let mut lines: Vec<String> = Vec::new();
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        match r.lines.recv_timeout(wait) {
            // output without a newline (an rc file's OSC 7 on `cd`) may
            // run into a marker or a match: they are found at the line end
            Ok(l) if l.ends_with(&end) => return Ok(lines),
            Ok(l) if l.ends_with(&begin) => inside = true,
            Ok(l) => {
                if let Some(at) = l
                    .find('\x1f')
                    .filter(|_| inside && lines.len() < MAX_ITEMS * 2)
                {
                    lines.push(l[at..].to_string());
                }
            }
            Err(RecvTimeoutError::Timeout) => return Err(Outcome::TimedOut),
            Err(RecvTimeoutError::Disconnected) => return Err(Outcome::Failed),
        }
    }
}

impl Drop for Completer {
    fn drop(&mut self) {
        self.stop();
    }
}

static COMPLETERS: LazyLock<Mutex<HashMap<String, Completer>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// [`Completer::complete`] on the warm shell for `program`, started on
/// first use. Call it from one completion thread: requests to one shell run
/// one at a time.
pub fn complete(program: &str, q: &Query, budget: Duration) -> Vec<Item> {
    let mut all = COMPLETERS.lock().unwrap_or_else(PoisonError::into_inner);
    all.entry(program.to_string())
        .or_insert_with(|| Completer::new(program))
        .complete(q, budget)
}

/// Stop every warm completion shell and what its completion functions
/// started (the setting was turned off, or the app quits).
pub fn stop_all() {
    COMPLETERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_sends_argument_words_only() {
        let q = query(ShellKind::Zsh, "mycmd de", 8, "/tmp").unwrap();
        assert_eq!(q.words, vec!["mycmd", "de"]);
        assert_eq!(q.line, "mycmd de");
        let q = query(ShellKind::Bash, "a | git 'che", 12, "/tmp").unwrap();
        assert_eq!(q.words, vec!["git", "che"], "current segment, unquoted");
        let q = query(ShellKind::Bash, "mycmd ", 6, "/tmp").unwrap();
        assert_eq!(q.words, vec!["mycmd", ""]);
        // the command name and path-like words are Rift's
        assert_eq!(query(ShellKind::Zsh, "myc", 3, "/tmp"), None);
        assert_eq!(query(ShellKind::Zsh, "ls src/m", 8, "/tmp"), None);
        assert_eq!(query(ShellKind::Zsh, "ls ~/", 5, "/tmp"), None);
    }

    #[test]
    fn query_refuses_substitutions_and_control_characters() {
        for line in [
            "ls $(touch /tmp/x) ",
            "ls `touch /tmp/x` ",
            "diff <(ls) ",
            "cat =(ls) ",
            "echo a\nb ",
            "echo \x1b[31m ",
        ] {
            assert_eq!(
                query(ShellKind::Zsh, line, line.len(), "/tmp"),
                None,
                "{line:?}"
            );
        }
        assert_eq!(query(ShellKind::Fish, "ls (pwd) ", 9, "/tmp"), None);
        // zsh glob qualifiers and parameter flags evaluate code
        for line in ["ls *(e:'touch x':) ", "ls *(+fn) ", "echo ${(e)v} "] {
            assert_eq!(
                query(ShellKind::Zsh, line, line.len(), "/tmp"),
                None,
                "{line:?}"
            );
        }
        // bash passes the words as data only
        assert!(query(ShellKind::Bash, "ls (pwd) ", 9, "/tmp").is_some());
        assert_eq!(query(ShellKind::Zsh, "ls a", 4, "/tm\np"), None);
        let long = format!("ls {}", "a".repeat(MAX_LINE));
        assert_eq!(query(ShellKind::Zsh, &long, long.len(), "/tmp"), None);
    }

    #[test]
    fn request_encoding_is_line_framed() {
        let q = Query {
            cwd: "/a b".into(),
            line: "git ch".into(),
            words: vec!["git".into(), "ch".into()],
        };
        assert_eq!(encode(7, &q), "7\n/a b\ngit ch\n1\ngit\nch\n");
    }

    #[test]
    fn replies_are_cleaned_and_bounded() {
        let long = "x".repeat(MAX_FIELD * 3);
        let lines = [
            "noise before".to_string(),
            "\x1fdeploy\x1fship it".to_string(),
            "\x1fdeploy\x1fdup".to_string(),
            "\x1f\x1b[31mred\x1b[0m\x1fa\x07b".to_string(),
            "\x1f\x1fempty word".to_string(),
            format!("\x1f{long}\x1f"),
        ];
        let items = parse_items(lines.iter().map(String::as_str));
        assert_eq!(items[0], ("deploy".into(), "ship it".into()));
        assert_eq!(items[1], ("red".into(), "ab".into()));
        assert_eq!(items[2].0.len(), MAX_FIELD);
        assert_eq!(items.len(), 3);
        let many: Vec<String> = (0..MAX_ITEMS * 2).map(|i| format!("\x1fw{i}")).collect();
        assert_eq!(
            parse_items(many.iter().map(String::as_str)).len(),
            MAX_ITEMS
        );
    }

    #[test]
    fn long_lines_are_cut_while_reading() {
        let (tx, rx) = channel();
        let big = format!("{}\nnext\n", "y".repeat(1 << 20));
        read_lines(std::io::Cursor::new(big.into_bytes()), tx);
        let got: Vec<String> = rx.iter().collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].len(), MAX_FIELD * 2 + 8);
        assert_eq!(got[1], "next");
    }
}
