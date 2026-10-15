//! Shell environment: aliases, functions, builtins; command validity and corrections.
//!
//! Backs three Warp-style editor features:
//!
//! * **Alias expansion** — [`ShellEnv::expand_alias_at`] rewrites an alias in
//!   command position into its definition (e.g. `ll ` → `ls -la `).
//! * **Command-validity highlighting** — [`ShellEnv::is_known`] plus the free
//!   function [`first_command_word_ranges`] let the renderer underline the
//!   command word of each pipeline stage when it resolves to nothing runnable.
//! * **Local command correction** — [`ShellEnv::suggest_correction`] proposes a
//!   fix for a mistyped command (`gti` → `git`, `git stauts` → `git status`)
//!   using edit distance only; no network, no AI.
//!
//! Aliases and function names are discovered by running the user's real shell
//! once, interactively, on a background thread (see [`ShellEnv::load`]). Every
//! parser is a pure function with unit tests; the shell probe has a hard
//! timeout and can never block forever or panic.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Which shell family a [`ShellEnv`] was loaded for. Chooses the probe command,
/// the alias-output grammar, and the static builtin/reserved-word list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
}

impl ShellKind {
    /// Classify by the basename of a shell program path. Anything that is not
    /// recognisably zsh or fish is treated as a POSIX/bash-compatible shell.
    pub fn from_program(shell_program: &str) -> ShellKind {
        let base = shell_program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(shell_program)
            .to_ascii_lowercase();
        if base.contains("zsh") {
            ShellKind::Zsh
        } else if base.contains("fish") {
            ShellKind::Fish
        } else {
            ShellKind::Bash
        }
    }
}

/// A snapshot of the interactive-shell command namespace: aliases, function
/// names, and builtins/reserved words. `$PATH` executables are *not* stored —
/// they are read on demand from [`crate::completion::path_commands`], which
/// scans `$PATH` once per process and caches the result.
#[derive(Debug, Clone)]
pub struct ShellEnv {
    shell: ShellKind,
    /// Alias name → expansion text (e.g. `"ll"` → `"ls -la"`).
    aliases: HashMap<String, String>,
    /// User/shell function names.
    functions: HashSet<String>,
    /// Builtins and reserved words for `shell`.
    builtins: HashSet<String>,
    /// Executables on the interactive shell's `$PATH` (sorted), which rc
    /// files extend (`PNPM_HOME`, nvm): Rift started from the Dock has a
    /// shorter one. Empty when the probe did not report it.
    path_cmds: Vec<String>,
}

/// Command prefixes that transparently precede the real command: the word after
/// them (and after any `VAR=val` assignments) is what actually runs.
const COMMAND_PREFIXES: &[&str] = &[
    "sudo", "doas", "env", "time", "nohup", "command", "exec", "builtin", "nice", "stdbuf",
    "setsid",
];

/// A compact list of common git subcommands, duplicated here (rather than
/// reaching into `completion.rs`) so command correction can fix `git stauts`
/// without a cross-module dependency.
const GIT_SUBCOMMANDS: &[&str] = &[
    "add",
    "am",
    "apply",
    "bisect",
    "blame",
    "branch",
    "checkout",
    "cherry-pick",
    "clean",
    "clone",
    "commit",
    "config",
    "diff",
    "fetch",
    "grep",
    "init",
    "log",
    "merge",
    "mv",
    "pull",
    "push",
    "rebase",
    "reflog",
    "remote",
    "reset",
    "restore",
    "revert",
    "rm",
    "show",
    "stash",
    "status",
    "switch",
    "tag",
    "worktree",
];

impl ShellEnv {
    /// Build an empty environment for `shell` (builtins populated, no aliases or
    /// functions). Used as the fallback when the probe cannot run.
    fn empty(shell: ShellKind) -> ShellEnv {
        ShellEnv {
            shell,
            aliases: HashMap::new(),
            functions: HashSet::new(),
            builtins: builtins_for(shell).iter().map(|s| s.to_string()).collect(),
            path_cmds: Vec::new(),
        }
    }

    /// `$PATH` executables: the shell's when the probe reported them, else
    /// Rift's own.
    fn path_commands(&self) -> &[String] {
        if self.path_cmds.is_empty() {
            crate::completion::path_commands()
        } else {
            &self.path_cmds
        }
    }

    /// Run `shell_program` once, interactively, to enumerate its aliases and
    /// function names, then merge in the static builtin list.
    ///
    /// The child gets a null stdin, a piped stdout, a null stderr, and
    /// `RIFT_ENV_PROBE=1` in its environment (so a user's rc file can special
    /// case us). A reader thread drains stdout while the caller polls
    /// [`std::process::Child::try_wait`]; if the shell has not exited within
    /// `timeout`, it is killed. This never blocks forever and never panics — a
    /// spawn failure or timeout simply yields an env with no aliases/functions.
    ///
    /// Intended to be called off the UI thread.
    pub fn load(shell_program: &str) -> ShellEnv {
        Self::load_with_timeout(shell_program, Duration::from_secs(3))
    }

    /// [`ShellEnv::load`] with an explicit probe timeout (exposed for tests).
    pub fn load_with_timeout(shell_program: &str, timeout: Duration) -> ShellEnv {
        let shell = ShellKind::from_program(shell_program);
        let mut env = ShellEnv::empty(shell);

        let Some(output) = run_probe(shell_program, shell, timeout) else {
            return env;
        };
        env.add_probe_output(&output);
        env
    }

    /// Merge the probe's output: whatever the rc files printed (an iTerm2
    /// integration's OSC 1337 reports, a greeting), then the start sentinel,
    /// the aliases, the functions sentinel and the function names.
    fn add_probe_output(&mut self, output: &str) {
        // rc output before the start sentinel is not ours: unterminated, it
        // glued itself to the first alias and hid it
        let output = output
            .split_once(PROBE_START)
            .map_or(output, |(_, rest)| rest);
        // the shell's $PATH comes last, after its own sentinel
        let output = match output.split_once(PROBE_PATH) {
            Some((rest, path)) => {
                let path = path.trim();
                if !path.is_empty() {
                    self.path_cmds = crate::completion::scan_path_commands(path.as_ref());
                    // command-name completions use it too
                    let _ = crate::completion::SHELL_PATH_COMMANDS.set(self.path_cmds.clone());
                }
                rest
            }
            None => output,
        };
        // Aliases and functions are emitted on either side of a sentinel line.
        let (alias_text, func_text) = match output.split_once(PROBE_SENTINEL) {
            Some((a, f)) => (a, f),
            None => (output, ""),
        };

        for (name, expansion) in parse_aliases(self.shell, alias_text) {
            self.aliases.insert(name, expansion);
        }
        for name in parse_function_names(func_text) {
            self.functions.insert(name);
        }
    }

    /// The shell family this environment was loaded for.
    pub fn shell(&self) -> ShellKind {
        self.shell
    }

    /// The alias definition for `word`, if `word` names an alias.
    pub fn alias(&self, word: &str) -> Option<&str> {
        self.aliases.get(word).map(String::as_str)
    }

    /// Whether `word` resolves to something runnable: an alias, function, or
    /// builtin/reserved word; a `$PATH` executable; a path-like word
    /// (`./x`, `~/x`, `a/b`) that exists and is executable; or an environment
    /// assignment such as `FOO=bar`.
    pub fn is_known(&self, word: &str) -> bool {
        if word.is_empty() {
            return false;
        }
        if is_assignment(word) {
            return true;
        }
        // `$EDITOR`, `${VAR}`, `$(cmd)`: only the shell can resolve these
        if word.contains('$') || word.contains('`') {
            return true;
        }
        if self.aliases.contains_key(word)
            || self.functions.contains(word)
            || self.builtins.contains(word)
        {
            return true;
        }
        if is_path_like(word) {
            return path_word_is_executable(word);
        }
        self.path_commands()
            .binary_search_by(|c| c.as_str().cmp(word))
            .is_ok()
    }

    /// Warp-style alias expansion for the word ending at (or just before)
    /// `cursor`. Returns the byte range to replace and the replacement text when
    /// that word is an alias sitting in **command position** (the first word of
    /// a pipeline stage, after skipping `VAR=val` assignments and prefixes like
    /// `sudo`). Returns `None` otherwise, and for a self-referential alias
    /// (`alias ls=ls`) so expansion cannot loop.
    pub fn expand_alias_at(&self, buffer: &str, cursor: usize) -> Option<(Range<usize>, String)> {
        let cursor = cursor.min(buffer.len());
        if !buffer.is_char_boundary(cursor) {
            return None;
        }
        // The word whose end is at (or immediately before) the cursor.
        let head = &buffer[..cursor];
        let word = last_token(head)?;
        // Only expand when that word is the command word of its segment.
        if !first_command_word_ranges(buffer).contains(&word) {
            return None;
        }
        let name = &buffer[word.clone()];
        let expansion = self.aliases.get(name)?;
        if expansion == name {
            return None;
        }
        Some((word, expansion.clone()))
    }

    /// A local correction for `command_line`, or `None` when it looks fine or
    /// nothing close enough is found. Two cases, in order:
    ///
    /// 1. **git subcommand typo** — `git stauts …` → `git status …`.
    /// 2. **unknown command** — the first command word does not resolve; the
    ///    nearest known command (alias, builtin, or `$PATH`) within
    ///    Damerau-Levenshtein distance 2 is substituted, preferring the same
    ///    first letter, then a smaller distance, then a shorter/simpler name.
    pub fn suggest_correction(&self, command_line: &str) -> Option<String> {
        let ranges = first_command_word_ranges(command_line);
        let cmd = ranges.first()?.clone();
        let word = &command_line[cmd.clone()];

        // Case 1: `git <typo>` — correct the subcommand, leave everything else.
        if word == "git" {
            if let Some(sub_range) = nth_word_range(command_line, cmd.end, 0) {
                let sub = &command_line[sub_range.clone()];
                if !sub.starts_with('-')
                    && !GIT_SUBCOMMANDS.contains(&sub)
                    && let Some(fixed) = correct_git_subcommand(sub)
                {
                    return Some(splice(command_line, sub_range, fixed));
                }
            }
            return None;
        }

        // Case 2: unknown first command word → nearest known command.
        if self.is_known(word) {
            return None;
        }
        let candidates = self.correction_candidates();
        let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
        let fixed = closest_command(word, &refs)?;
        Some(splice(command_line, cmd, fixed))
    }

    /// Candidate command names for correction: aliases, functions, builtins, and
    /// every `$PATH` executable.
    fn correction_candidates(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        out.extend(self.aliases.keys().cloned());
        out.extend(self.functions.iter().cloned());
        out.extend(self.builtins.iter().cloned());
        out.extend(self.path_commands().iter().cloned());
        out
    }
}

/// Sentinel line printed by the probe between the alias listing and the
/// function-name listing so the two can be parsed with the right grammar.
const PROBE_SENTINEL: &str = "__RIFT_ENV_PROBE_FUNCS__";
/// Printed first: everything before it came from the rc files.
const PROBE_START: &str = "__RIFT_ENV_PROBE_START__";
/// Printed before the shell's `$PATH` (colon-separated), the last line.
const PROBE_PATH: &str = "__RIFT_ENV_PROBE_PATH__";

/// The shell command string that lists aliases, then the sentinel, then
/// function names, one per line, then the `$PATH` sentinel and `$PATH`.
fn probe_script(shell: ShellKind) -> &'static str {
    match shell {
        // zsh: `alias` prints `name=value` (value single-quoted when needed);
        // `${(k)functions}` are the function names.
        ShellKind::Zsh => {
            "print -r -- __RIFT_ENV_PROBE_START__; alias; print -r -- __RIFT_ENV_PROBE_FUNCS__; \
             print -rl -- ${(k)functions}; print -r -- __RIFT_ENV_PROBE_PATH__; print -r -- \"$PATH\""
        }
        // bash: `alias` prints `alias name='value'`; `compgen -A function` the
        // function names.
        ShellKind::Bash => {
            "echo __RIFT_ENV_PROBE_START__; alias; echo __RIFT_ENV_PROBE_FUNCS__; \
             compgen -A function; echo __RIFT_ENV_PROBE_PATH__; echo \"$PATH\""
        }
        // fish: `alias` prints `alias name value`; `functions -n` the names;
        // `$PATH` is a list there
        ShellKind::Fish => {
            "echo __RIFT_ENV_PROBE_START__; alias; echo __RIFT_ENV_PROBE_FUNCS__; functions -n; \
             echo __RIFT_ENV_PROBE_PATH__; string join : $PATH"
        }
    }
}

/// Spawn the shell, run the probe script, and return its stdout — or `None` on
/// spawn failure. Kills the child on timeout; never blocks past `timeout` plus a
/// short drain grace period.
fn run_probe(shell_program: &str, shell: ShellKind, timeout: Duration) -> Option<String> {
    let mut cmd = Command::new(shell_program);
    match shell {
        // Interactive (`-i`) so rc files defining aliases/functions are sourced;
        // `-c` runs our script. fish takes no `-i` with `-c`.
        ShellKind::Fish => {
            cmd.arg("-c").arg(probe_script(shell));
        }
        _ => {
            cmd.arg("-i").arg("-c").arg(probe_script(shell));
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("RIFT_ENV_PROBE", "1");

    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;

    // Drain stdout on a thread so a full pipe buffer cannot deadlock the poll.
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                break;
            }
        }
    }
    let _ = child.wait();

    // The reader returns promptly once the pipe closes (exit or kill).
    let out = rx.recv_timeout(Duration::from_secs(1)).ok();
    let _ = reader.join();
    out
}

/// Parse alias-listing text for `shell` into `(name, expansion)` pairs.
pub fn parse_aliases(shell: ShellKind, text: &str) -> Vec<(String, String)> {
    match shell {
        ShellKind::Zsh => parse_zsh_aliases(text),
        ShellKind::Bash => parse_bash_aliases(text),
        ShellKind::Fish => parse_fish_aliases(text),
    }
}

/// Parse zsh `alias` output: lines of `name=value`, where `value` is either bare
/// or single-quoted with `'\''` standing in for an embedded single quote.
/// Global (`alias -g`) and suffix (`alias -s`) forms are not emitted by a plain
/// `alias`, so nothing special is needed for them.
pub fn parse_zsh_aliases(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if !is_alias_name(name) {
            continue;
        }
        out.push((name.to_string(), unquote_shell(value)));
    }
    out
}

/// Parse bash `alias` output: lines of `alias name='value'` (value always
/// single-quoted, `'\''` for an embedded single quote).
pub fn parse_bash_aliases(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("alias ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        if !is_alias_name(name) {
            continue;
        }
        out.push((name.to_string(), unquote_shell(value)));
    }
    out
}

/// Parse fish `alias` output. fish prints `alias name value` or
/// `alias name=value`; the value may be single- or double-quoted.
pub fn parse_fish_aliases(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("alias ") else {
            continue;
        };
        // `name=value` form.
        let (name, value) = if let Some((n, v)) = split_before_unquoted_eq(rest) {
            (n, v)
        } else if let Some((n, v)) = rest.split_once(char::is_whitespace) {
            // `name value` form.
            (n.to_string(), v.trim().to_string())
        } else {
            continue;
        };
        if !is_alias_name(&name) {
            continue;
        }
        out.push((name, unquote_shell(&value)));
    }
    out
}

/// Function names from the probe's function listing: non-empty trimmed lines
/// (also tolerating comma/space separation, which some `functions` builds use).
fn parse_function_names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        for name in line.split([',', ' ', '\t']) {
            let name = name.trim();
            if !name.is_empty() && !name.starts_with('#') {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// Byte ranges of the command word of each pipeline stage in `buffer`, in order.
///
/// The buffer is split on the unquoted separators `;` `|` `&` `&&` `||` and
/// newlines (and `(`/`)` subshell delimiters). Within each stage, leading
/// `VAR=val` assignments and transparent prefixes (`sudo`, `env`, `time`,
/// `nohup`, …) are skipped; the next word's range is recorded. Stages with no
/// command word (empty, or only assignments) contribute nothing.
pub fn first_command_word_ranges(buffer: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for (start, end) in segment_bounds(buffer) {
        let seg = &buffer[start..end];
        // the transparent prefix whose options are being skipped, if any
        let mut prefix: Option<&str> = None;
        let mut skip_value = false;
        let mut options_done = false;
        for tok in token_ranges(seg, start) {
            let word = &buffer[tok.clone()];
            if skip_value {
                skip_value = false;
                continue;
            }
            if let Some(p) = prefix
                && !options_done
                && word.starts_with('-')
            {
                if word == "--" {
                    options_done = true;
                } else {
                    skip_value = prefix_option_takes_value(p, word);
                }
                continue;
            }
            if is_assignment(word) {
                continue;
            }
            if let Some(p) = COMMAND_PREFIXES.iter().find(|p| **p == word) {
                prefix = Some(p);
                options_done = false;
                continue;
            }
            out.push(tok);
            break;
        }
    }
    out
}

/// Whether `opt` (an option of the transparent prefix `prefix`) takes its
/// value as the next word (`sudo -u bob cmd`, `env -u HOME cmd`), so that
/// word is not mistaken for the command.
fn prefix_option_takes_value(prefix: &str, opt: &str) -> bool {
    let takes: &[&str] = match prefix {
        "sudo" => &[
            "-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-U", "-r", "-t",
        ],
        "doas" => &["-u", "-C"],
        "env" => &["-u", "-C", "-S", "-P"],
        "nice" => &["-n"],
        "stdbuf" => &["-i", "-o", "-e"],
        "exec" => &["-a"],
        _ => &[],
    };
    takes.contains(&opt)
}

/// Damerau-Levenshtein (optimal string alignment) distance between two strings,
/// counting a single insertion, deletion, substitution, or transposition of
/// adjacent characters as cost 1.
pub fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let width = m + 1;
    // Three rolling rows: prev-prev, prev, current.
    let mut d0 = vec![0usize; width];
    let mut d1: Vec<usize> = (0..=m).collect();
    let mut d2 = vec![0usize; width];
    for i in 1..=n {
        d2[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut val = (d1[j] + 1).min(d2[j - 1] + 1).min(d1[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                val = val.min(d0[j - 2] + 1);
            }
            d2[j] = val;
        }
        std::mem::swap(&mut d0, &mut d1);
        std::mem::swap(&mut d1, &mut d2);
    }
    d1[m]
}

/// The closest command in `candidates` to `word` within Damerau-Levenshtein
/// distance 2, or `None`. Ties break toward the same first letter, then a
/// smaller distance, then a shorter name, then lexicographic order. Never
/// returns `word` itself.
pub fn closest_command<'a>(word: &str, candidates: &[&'a str]) -> Option<&'a str> {
    if word.is_empty() {
        return None;
    }
    let word_first = word.chars().next();
    let mut best: Option<(bool, usize, usize, &'a str)> = None;
    for &cand in candidates {
        if cand == word || cand.is_empty() {
            continue;
        }
        // Cheap length gate: distance is at least the length difference.
        if cand.len().abs_diff(word.len()) > 2 {
            continue;
        }
        let dist = damerau_levenshtein(word, cand);
        if dist == 0 || dist > 2 {
            continue;
        }
        // Same first letter is the primary preference (per spec), then a
        // smaller distance, then a shorter/simpler name.
        let diff_first = cand.chars().next() != word_first;
        let key = (diff_first, dist, cand.len(), cand);
        match &best {
            Some((bd, bf, bl, bc)) if (*bd, *bf, *bl, *bc) <= key => {}
            _ => best = Some(key),
        }
    }
    best.map(|(_, _, _, cand)| cand)
}

/// The closest git subcommand to `sub` within Damerau-Levenshtein distance 2.
pub fn correct_git_subcommand(sub: &str) -> Option<&'static str> {
    if sub.is_empty() || GIT_SUBCOMMANDS.contains(&sub) {
        return None;
    }
    // `GIT_SUBCOMMANDS` is `&'static [&'static str]`, so the borrow is 'static.
    closest_command(sub, GIT_SUBCOMMANDS)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Static builtin + reserved-word list for a shell family.
fn builtins_for(shell: ShellKind) -> &'static [&'static str] {
    const POSIX: &[&str] = &[
        // builtins common to bash/zsh/sh
        ".", ":", "[", "alias", "bg", "bind", "break", "builtin", "cd", "command", "continue",
        "declare", "dirs", "disown", "echo", "enable", "eval", "exec", "exit", "export", "false",
        "fc", "fg", "getopts", "hash", "help", "history", "jobs", "kill", "let", "local", "logout",
        "popd", "printf", "pushd", "pwd", "read", "readonly", "return", "set", "shift", "source",
        "suspend", "test", "times", "trap", "true", "type", "typeset", "ulimit", "umask",
        "unalias", "unset", "wait", // reserved words
        "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done", "case", "esac",
        "function", "select", "in", "!", "{", "}", "[[", "]]",
    ];
    const BASH_EXTRA: &[&str] = &[
        "compgen",
        "complete",
        "compopt",
        "caller",
        "coproc",
        "mapfile",
        "readarray",
        "shopt",
        "time",
    ];
    const ZSH_EXTRA: &[&str] = &[
        "autoload",
        "bindkey",
        "compdef",
        "emulate",
        "print",
        "setopt",
        "unsetopt",
        "vared",
        "whence",
        "where",
        "which",
        "zcompile",
        "zle",
        "zmodload",
        "zstyle",
        "zparseopts",
        "foreach",
        "end",
        "nocorrect",
        "noglob",
    ];
    const FISH: &[&str] = &[
        "and",
        "or",
        "not",
        "begin",
        "end",
        "if",
        "else",
        "switch",
        "case",
        "while",
        "for",
        "in",
        "function",
        "return",
        "break",
        "continue",
        "set",
        "read",
        "echo",
        "printf",
        "test",
        "count",
        "string",
        "math",
        "abbr",
        "alias",
        "bind",
        "builtin",
        "command",
        "complete",
        "contains",
        "eval",
        "exec",
        "exit",
        "functions",
        "history",
        "jobs",
        "pwd",
        "source",
        "status",
        "type",
        "ulimit",
        "umask",
        "cd",
        "commandline",
        "fish_config",
        "funced",
        "funcsave",
        "argparse",
        "true",
        "false",
        "wait",
    ];
    // Combined bash/zsh lists are built once and cached (POSIX + the family
    // extras); fish is a single literal.
    use std::sync::OnceLock;
    static BASH: OnceLock<Vec<&'static str>> = OnceLock::new();
    static ZSH: OnceLock<Vec<&'static str>> = OnceLock::new();
    match shell {
        ShellKind::Bash => BASH.get_or_init(|| [POSIX, BASH_EXTRA].concat()).as_slice(),
        ShellKind::Zsh => ZSH.get_or_init(|| [POSIX, ZSH_EXTRA].concat()).as_slice(),
        ShellKind::Fish => FISH,
    }
}

/// A valid alias/identifier name for our purposes: non-empty, no whitespace, no
/// `=`, and not starting with `-` (which would be an option like `alias -g`).
fn is_alias_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && !name.starts_with('-')
        && !name.contains(char::is_whitespace)
        && !name.contains('=')
}

/// Strip one layer of shell quoting from an alias value:
/// `'ls -la'` → `ls -la`, `'a'\''b'` → `a'b`, `"x"` → `x`, bare stays bare.
fn unquote_shell(value: &str) -> String {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        // zsh/bash single-quote escaping: '\'' means a literal single quote.
        return inner.replace("'\\''", "'");
    }
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return inner.replace("\\\"", "\"").replace("\\\\", "\\");
    }
    value.to_string()
}

/// Split `name=value` at the first `=` that is not inside quotes, returning the
/// name and the raw value. `None` when there is no such `=`.
fn split_before_unquoted_eq(s: &str) -> Option<(String, String)> {
    let (mut in_s, mut in_d) = (false, false);
    for (i, c) in s.char_indices() {
        match c {
            '\'' if !in_d => in_s = !in_s,
            '"' if !in_s => in_d = !in_d,
            '=' if !in_s && !in_d => {
                return Some((s[..i].to_string(), s[i + 1..].to_string()));
            }
            _ => {}
        }
    }
    None
}

/// Whether `word` is an environment assignment `NAME=...` (NAME a shell
/// identifier: leading letter/underscore, then alnum/underscore).
fn is_assignment(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    if eq == 0 {
        return false;
    }
    let name = &word[..eq];
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `word` names a filesystem path rather than a bare command.
fn is_path_like(word: &str) -> bool {
    word.contains('/') || word.starts_with('.') || word.starts_with('~')
}

/// Whether a path-like `word` points at an existing executable file. `~/` is
/// expanded against `$HOME`; relative paths resolve against the process cwd
/// (best effort — the editor does not track the pane's cwd here).
fn path_word_is_executable(word: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let expanded = if let Some(rest) = word.strip_prefix("~/") {
        match std::env::var("HOME") {
            Ok(home) => format!("{home}/{rest}"),
            Err(_) => return false,
        }
    } else {
        word.to_string()
    };
    std::fs::metadata(&expanded)
        .map(|m| !m.is_dir() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Byte range of the last whitespace-delimited (quote-aware) token in `s`, or
/// `None` when `s` has no token.
fn last_token(s: &str) -> Option<Range<usize>> {
    token_ranges(s, 0).into_iter().next_back()
}

/// Byte range of the `n`th token (0-based) at or after byte offset `from` in
/// `buffer`. Used to reach the git subcommand after `git`.
fn nth_word_range(buffer: &str, from: usize, n: usize) -> Option<Range<usize>> {
    if from > buffer.len() || !buffer.is_char_boundary(from) {
        return None;
    }
    token_ranges(&buffer[from..], from).into_iter().nth(n)
}

/// Quote-aware whitespace tokenization: byte ranges (offset by `base`) of each
/// run of non-whitespace, where whitespace inside single/double quotes does not
/// split. Separator characters are not treated specially (callers pre-split on
/// them via [`segment_bounds`]).
fn token_ranges(seg: &str, base: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let (mut in_s, mut in_d) = (false, false);
    for (i, c) in seg.char_indices() {
        let is_ws = c.is_whitespace() && !in_s && !in_d;
        if is_ws {
            if let Some(st) = start.take() {
                out.push(base + st..base + i);
            }
        } else {
            if start.is_none() {
                start = Some(i);
            }
            if c == '\'' && !in_d {
                in_s = !in_s;
            } else if c == '"' && !in_s {
                in_d = !in_d;
            }
        }
    }
    if let Some(st) = start {
        out.push(base + st..base + seg.len());
    }
    out
}

/// Byte ranges of pipeline stages: `buffer` split on the unquoted shell command
/// separators `;` `|` `&` `\n` `(` `)` (and the doubled `&&`/`||`); the `&` of a
/// redirection (`2>&1`, `&>file`) is not a separator. Empty spans are omitted.
fn segment_bounds(buffer: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let (mut in_s, mut in_d) = (false, false);
    let bytes = buffer.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '\'' if !in_d => in_s = !in_s,
            '"' if !in_s => in_d = !in_d,
            _ if in_s || in_d => {}
            ';' | '\n' | '(' | ')' => {
                if start < i {
                    out.push((start, i));
                }
                start = i + 1;
            }
            // `2>&1`, `>&2`, `<&3`, `&>file`, `&>>file`: redirections, not `&`
            '&' if (i > 0 && matches!(bytes[i - 1], b'>' | b'<'))
                || matches!(bytes.get(i + 1), Some(b'>')) => {}
            '|' | '&' => {
                if start < i {
                    out.push((start, i));
                }
                // Consume a doubled operator (`&&`, `||`) as one separator.
                if i + 1 < bytes.len() && bytes[i + 1] == bytes[i] {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < buffer.len() {
        out.push((start, buffer.len()));
    }
    out
}

/// Replace `range` in `buffer` with `replacement`, returning the new string.
fn splice(buffer: &str, range: Range<usize>, replacement: &str) -> String {
    let mut out = String::with_capacity(buffer.len() + replacement.len());
    out.push_str(&buffer[..range.start]);
    out.push_str(replacement);
    out.push_str(&buffer[range.end..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(aliases: &[(&str, &str)], builtins: &[&str]) -> ShellEnv {
        ShellEnv {
            shell: ShellKind::Zsh,
            aliases: aliases
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            functions: HashSet::new(),
            builtins: builtins.iter().map(|s| s.to_string()).collect(),
            path_cmds: Vec::new(),
        }
    }

    #[test]
    fn shell_kind_from_program() {
        assert_eq!(ShellKind::from_program("/bin/zsh"), ShellKind::Zsh);
        assert_eq!(ShellKind::from_program("/usr/bin/fish"), ShellKind::Fish);
        assert_eq!(ShellKind::from_program("/bin/bash"), ShellKind::Bash);
        assert_eq!(ShellKind::from_program("bash"), ShellKind::Bash);
        assert_eq!(ShellKind::from_program("sh"), ShellKind::Bash);
        assert_eq!(ShellKind::from_program("-zsh"), ShellKind::Zsh);
    }

    /// An iTerm2 shell integration in `.zshrc` prints its OSC 1337 reports
    /// with no newline (as a real `zsh -i -c alias` showed): the first alias
    /// must still be read. A stand-in `zsh` prints them, then runs the probe.
    #[test]
    fn rc_output_before_the_probe_is_skipped() {
        if !std::path::Path::new("/bin/zsh").exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("rift-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shell = dir.join("zsh");
        // the rc file also puts a tool directory on PATH (PNPM_HOME)
        let tools = dir.join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join("rift-probe-tool"), "#!/bin/sh\n").unwrap();
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\n\
                 printf '\\033]1337;CurrentDir=/tmp\\007\\033]1337;ShellIntegrationVersion=14;shell=zsh\\007'\n\
                 export PATH=\"{}:$PATH\"\n\
                 exec /bin/zsh -f -c \"alias build='docker compose up --build'; alias up=x; $3\"\n",
                tools.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(
            tools.join("rift-probe-tool"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let env = ShellEnv::load_with_timeout(shell.to_str().unwrap(), Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(env.alias("build"), Some("docker compose up --build"));
        assert_eq!(env.alias("up"), Some("x"));
        // a command only the shell's PATH has is known
        assert!(env.is_known("rift-probe-tool"));
        assert!(!env.is_known("rift-probe-missing"));
    }

    #[test]
    fn parse_zsh_alias_forms() {
        let text = "ll='ls -la'\ng=git\ngp='git push'\ngrep='grep --color=auto'\nquote='echo '\\''hi'\\'''";
        let map: HashMap<_, _> = parse_zsh_aliases(text).into_iter().collect();
        assert_eq!(map.get("ll").map(String::as_str), Some("ls -la"));
        assert_eq!(map.get("g").map(String::as_str), Some("git"));
        assert_eq!(map.get("gp").map(String::as_str), Some("git push"));
        assert_eq!(
            map.get("grep").map(String::as_str),
            Some("grep --color=auto")
        );
        assert_eq!(map.get("quote").map(String::as_str), Some("echo 'hi'"));
    }

    #[test]
    fn parse_zsh_ignores_non_alias_lines() {
        // A stray blank / non-`name=value` line must not become an alias.
        let out = parse_zsh_aliases("\n-g globalalias='x'\n= nope\n");
        assert!(out.iter().all(|(n, _)| n != "-g" && !n.is_empty()));
    }

    #[test]
    fn parse_bash_alias_forms() {
        let text = "alias ll='ls -la'\nalias gs='git status'\nalias q='echo '\\''hi'\\'''";
        let map: HashMap<_, _> = parse_bash_aliases(text).into_iter().collect();
        assert_eq!(map.get("ll").map(String::as_str), Some("ls -la"));
        assert_eq!(map.get("gs").map(String::as_str), Some("git status"));
        assert_eq!(map.get("q").map(String::as_str), Some("echo 'hi'"));
        // lines that are not `alias name=...` are ignored
        assert!(parse_bash_aliases("not an alias line").is_empty());
    }

    #[test]
    fn parse_fish_alias_forms() {
        let text = "alias ll 'ls -la'\nalias g git\nalias gp='git push'";
        let map: HashMap<_, _> = parse_fish_aliases(text).into_iter().collect();
        assert_eq!(map.get("ll").map(String::as_str), Some("ls -la"));
        assert_eq!(map.get("g").map(String::as_str), Some("git"));
        assert_eq!(map.get("gp").map(String::as_str), Some("git push"));
    }

    #[test]
    fn assignment_detection() {
        assert!(is_assignment("FOO=bar"));
        assert!(is_assignment("_x=1"));
        assert!(is_assignment("PATH="));
        assert!(!is_assignment("=bar"));
        assert!(!is_assignment("1FOO=bar"));
        assert!(!is_assignment("foo"));
        assert!(!is_assignment("a-b=c"));
    }

    #[test]
    fn is_known_sources() {
        let env = env_with(&[("ll", "ls -la")], &["cd", "export"]);
        assert!(env.is_known("ll")); // alias
        assert!(env.is_known("cd")); // builtin
        assert!(env.is_known("FOO=bar")); // assignment
        assert!(!env.is_known("definitely-not-a-command-xyz"));
        assert!(!env.is_known(""));
    }

    #[test]
    fn is_known_path_like_executable() {
        let env = env_with(&[], &[]);
        assert!(env.is_known("/bin/sh")); // exists + executable on every unix
        assert!(!env.is_known("./nonexistent-xyz-shift"));
    }

    #[test]
    fn command_word_ranges_basic() {
        let ranges = first_command_word_ranges("ls -la");
        assert_eq!(ranges, vec![0..2]);
        assert_eq!(&"ls -la"[ranges[0].clone()], "ls");
    }

    #[test]
    fn command_word_ranges_skips_assignments_and_prefixes() {
        let buf = "FOO=bar sudo apt-get install";
        let ranges = first_command_word_ranges(buf);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&buf[ranges[0].clone()], "apt-get");
    }

    #[test]
    fn command_word_ranges_across_pipeline() {
        let buf = "cat f | grep x && sort";
        let words: Vec<&str> = first_command_word_ranges(buf)
            .into_iter()
            .map(|r| &buf[r])
            .collect();
        assert_eq!(words, vec!["cat", "grep", "sort"]);
    }

    #[test]
    fn command_word_ranges_quote_aware() {
        // The quoted assignment value must not split into a bogus command word.
        let buf = "FOO='a b' run";
        let words: Vec<&str> = first_command_word_ranges(buf)
            .into_iter()
            .map(|r| &buf[r])
            .collect();
        assert_eq!(words, vec!["run"]);
    }

    fn cmd_words(buf: &str) -> Vec<&str> {
        first_command_word_ranges(buf)
            .into_iter()
            .map(|r| &buf[r])
            .collect()
    }

    #[test]
    fn redirections_do_not_split_the_segment() {
        assert_eq!(cmd_words("cmd 2>&1"), vec!["cmd"]);
        assert_eq!(cmd_words("cmd >&2"), vec!["cmd"]);
        assert_eq!(cmd_words("cmd &> out.log"), vec!["cmd"]);
        assert_eq!(cmd_words("cmd &>> out.log"), vec!["cmd"]);
        assert_eq!(cmd_words("cmd <&3"), vec!["cmd"]);
        assert_eq!(cmd_words("a 2>&1 | b && c &"), vec!["a", "b", "c"]);
        assert_eq!(cmd_words("sleep 1 & echo hi"), vec!["sleep", "echo"]);
    }

    #[test]
    fn prefix_options_are_not_command_words() {
        assert_eq!(cmd_words("sudo -u bob cmd arg"), vec!["cmd"]);
        assert_eq!(cmd_words("sudo -E -n cmd"), vec!["cmd"]);
        assert_eq!(cmd_words("sudo -- cmd"), vec!["cmd"]);
        assert_eq!(cmd_words("env -i cmd"), vec!["cmd"]);
        assert_eq!(cmd_words("env -u HOME FOO=1 cmd"), vec!["cmd"]);
        assert_eq!(cmd_words("nice -n 10 cmd"), vec!["cmd"]);
        assert_eq!(cmd_words("sudo env -i FOO=1 cmd"), vec!["cmd"]);
        assert_eq!(cmd_words("time -p cmd"), vec!["cmd"]);
        // an option of the command itself is untouched
        assert_eq!(cmd_words("cmd -u bob"), vec!["cmd"]);
    }

    #[test]
    fn variable_command_words_are_known() {
        let env = env_with(&[], &[]);
        assert!(env.is_known("$EDITOR"));
        assert!(env.is_known("${EDITOR}"));
        assert!(env.is_known("\"$cmd\""));
        assert!(!env.is_known("definitely-not-a-command-xyz"));
        assert_eq!(env.suggest_correction("$EDITR file"), None);
    }

    #[test]
    fn command_word_ranges_empty_and_assignment_only() {
        assert!(first_command_word_ranges("").is_empty());
        assert!(first_command_word_ranges("   ").is_empty());
        assert!(first_command_word_ranges("FOO=bar").is_empty());
    }

    #[test]
    fn expand_alias_command_position() {
        let env = env_with(&[("ll", "ls -la"), ("g", "git")], &[]);
        // at end of the alias word
        assert_eq!(
            env.expand_alias_at("ll", 2),
            Some((0..2, "ls -la".to_string()))
        );
        // trailing space still targets the alias word
        assert_eq!(
            env.expand_alias_at("ll ", 3),
            Some((0..2, "ls -la".to_string()))
        );
        // after a pipe: still command position
        assert_eq!(
            env.expand_alias_at("cat x | g", 9),
            Some((8..9, "git".to_string()))
        );
    }

    #[test]
    fn expand_alias_not_in_command_position() {
        let env = env_with(&[("ll", "ls -la")], &[]);
        // `ll` as an argument, not the command
        assert_eq!(env.expand_alias_at("echo ll", 7), None);
        // not an alias
        assert_eq!(env.expand_alias_at("git st", 6), None);
    }

    #[test]
    fn expand_alias_self_reference_is_noop() {
        let env = env_with(&[("ls", "ls")], &[]);
        assert_eq!(env.expand_alias_at("ls", 2), None);
    }

    #[test]
    fn damerau_distance() {
        assert_eq!(damerau_levenshtein("", ""), 0);
        assert_eq!(damerau_levenshtein("git", "git"), 0);
        assert_eq!(damerau_levenshtein("gti", "git"), 1); // transposition
        assert_eq!(damerau_levenshtein("stauts", "status"), 1); // transposition
        assert_eq!(damerau_levenshtein("kitten", "sitting"), 3);
        assert_eq!(damerau_levenshtein("abc", ""), 3);
    }

    #[test]
    fn closest_prefers_same_first_letter_then_distance() {
        let cands = ["git", "grep", "gzip", "ls"];
        assert_eq!(closest_command("gti", &cands), Some("git")); // transposition, dist 1
        // same distance to two, prefer same first letter as typed
        assert_eq!(closest_command("gz", &["az", "gzip"]), Some("gzip"));
        // nothing within distance 2
        assert_eq!(closest_command("zzzzzz", &cands), None);
        // never returns the word itself (even when it is present in the list)
        assert_ne!(closest_command("git", &cands), Some("git"));
        assert_eq!(closest_command("ls", &["ls"]), None);
    }

    #[test]
    fn git_subcommand_correction() {
        assert_eq!(correct_git_subcommand("stauts"), Some("status"));
        assert_eq!(correct_git_subcommand("comit"), Some("commit"));
        assert_eq!(correct_git_subcommand("chekout"), Some("checkout"));
        assert_eq!(correct_git_subcommand("status"), None); // already valid
        assert_eq!(correct_git_subcommand("qqqqqqq"), None); // nothing close
    }

    #[test]
    fn suggest_correction_git_typo() {
        let env = env_with(&[], &[]);
        assert_eq!(
            env.suggest_correction("git stauts"),
            Some("git status".to_string())
        );
        assert_eq!(
            env.suggest_correction("git comit -m 'x'"),
            Some("git commit -m 'x'".to_string())
        );
        // valid subcommand → no correction
        assert_eq!(env.suggest_correction("git status"), None);
        // an option, not a subcommand typo
        assert_eq!(env.suggest_correction("git --version"), None);
    }

    #[test]
    fn suggest_correction_unknown_command() {
        // `sttatus` is unknown; nearest candidate is our alias `status`.
        let env = env_with(&[("status", "git status")], &["cd"]);
        assert_eq!(
            env.suggest_correction("sttatus"),
            Some("status".to_string())
        );
        // a known command is left alone
        assert_eq!(env.suggest_correction("cd /tmp"), None);
    }

    #[test]
    fn suggest_correction_preserves_arguments() {
        let env = env_with(&[("export", "export")], &["export"]);
        // `exprot` → `export`, keeping the rest of the line
        assert_eq!(
            env.suggest_correction("exprot FOO=bar"),
            Some("export FOO=bar".to_string())
        );
    }

    #[test]
    fn unquote_variants() {
        assert_eq!(unquote_shell("'ls -la'"), "ls -la");
        assert_eq!(unquote_shell("bare"), "bare");
        assert_eq!(unquote_shell("\"quoted\""), "quoted");
        assert_eq!(unquote_shell("'a'\\''b'"), "a'b");
    }

    #[test]
    fn segment_bounds_respects_quotes() {
        // `;` inside quotes is not a separator.
        let buf = "echo 'a;b' | wc";
        let segs = segment_bounds(buf);
        assert_eq!(segs.len(), 2);
        assert_eq!(&buf[segs[0].0..segs[0].1], "echo 'a;b' ");
    }

    // Optional: exercise the real shell probe, skipped when the shell is absent.
    #[test]
    fn load_real_shell_smoke() {
        let shell = if std::path::Path::new("/bin/zsh").exists() {
            "/bin/zsh"
        } else if std::path::Path::new("/bin/bash").exists() {
            "/bin/bash"
        } else {
            return; // no shell available; skip
        };
        // Must return promptly and never panic; builtins are always populated.
        let env = ShellEnv::load_with_timeout(shell, Duration::from_secs(5));
        assert!(env.is_known("cd"));
        assert!(env.is_known("echo"));
    }

    #[test]
    fn load_missing_shell_is_graceful() {
        let env = ShellEnv::load("/no/such/shell/rift-xyz");
        // spawn failed → still a usable env with builtins
        assert!(env.aliases.is_empty());
        assert!(env.is_known("cd"));
    }
}
