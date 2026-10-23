/// Completion providers for the input editor (design 04): PATH commands,
/// spec-driven subcommands / flags / argument generators (see
/// [`crate::completion_specs`]), filesystem paths, and shell history,
/// ranked by [`Match`] (see [`complete`]). Directory listings, `$PATH` scans
/// and generator inputs are cached until the files they came from change
/// ([`crate::completion_cache`]).
use crate::completion_cache::Stamp;
use crate::completion_specs::{self, ArgKind};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// Full replacement for the token being completed.
    pub insert: String,
    pub description: String,
    pub source: &'static str,
}

/// (start_byte, token) being completed — the shell word containing
/// `cursor`, as typed. Whitespace or an operator (`|;&<>()`) ends the word
/// (as in [`token_end`]) unless escaped with `\` or inside quotes.
pub fn current_token(buffer: &str, cursor: usize) -> (usize, &str) {
    let before = &buffer[..cursor];
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in before.char_indices() {
        if escaped {
            escaped = false;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_some() {
            escaped = c == '\\' && quote == Some('"');
        } else if c == '\\' {
            escaped = true;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c.is_whitespace() || "|;&<>()".contains(c) {
            start = i + c.len_utf8();
        }
    }
    (start, &before[start..])
}

/// End byte of the shell word containing `cursor` (see [`current_token`]):
/// the first unquoted, unescaped whitespace or `|;&<>()` at or after
/// `cursor`. Completion replaces `start..end`, so a closing quote or the
/// rest of a word the cursor sits inside is replaced, not kept.
pub fn token_end(buffer: &str, cursor: usize) -> usize {
    let (start, _) = current_token(buffer, cursor);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in buffer[start..].char_indices() {
        let at = start + i;
        if escaped {
            escaped = false;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_some() {
            escaped = c == '\\' && quote == Some('"');
        } else if c == '\\' {
            escaped = true;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if at >= cursor && (c.is_whitespace() || "|;&<>()".contains(c)) {
            return at;
        }
    }
    buffer.len()
}

/// The literal text of a typed shell word: quotes removed, `\x` → `x`.
pub(crate) fn unescape_word(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    let mut quote: Option<char> = None;
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            // inside "…" a backslash escapes only `$ ` " \` and newline
            // (bash, zsh and fish alike); before anything else it is literal
            (Some('"'), '\\') => match chars.clone().next() {
                Some(n) if "$`\"\\\n".contains(n) => {
                    chars.next();
                    out.push(n);
                }
                _ => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    out
}

/// Backslash-escape `s` as one bash/zsh/fish word. A leading `~/` stays
/// unescaped so it still expands. `None` for control characters, which no
/// form portable across the three shells can carry.
pub fn escape_word(s: &str) -> Option<String> {
    if s.chars().any(char::is_control) {
        return None;
    }
    let (lead, rest) = match s.strip_prefix("~/") {
        Some(rest) => ("~/", rest),
        None => ("", s),
    };
    let mut out = String::from(lead);
    for (i, c) in rest.char_indices() {
        let at_start = lead.is_empty() && i == 0;
        let safe =
            c.is_alphanumeric() || "/._-+,:@%".contains(c) || (!at_start && "=~".contains(c));
        if !safe {
            out.push('\\');
        }
        out.push(c);
    }
    Some(out)
}

/// [`escape_word`] for a command-position word. A name shaped like an
/// assignment (`NAME=…`) would set a shell variable there instead of running
/// the file, so its first `=` is escaped too (bash, zsh and fish then run
/// it). Reserved-word names (`time`, `[[`) stay bare on purpose: the shell's
/// keyword is what someone typing them means.
pub fn escape_command(s: &str) -> Option<String> {
    let word = escape_word(s)?;
    let Some((name, _)) = s.split_once('=') else {
        return Some(word);
    };
    let is_name = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !is_name {
        return Some(word);
    }
    // the name part needs no escapes, so its `=` is at the same offset
    Some(format!("{name}\\{}", &word[name.len()..]))
}

/// How often a cached `$PATH` scan checks its directories for changes.
const PATH_RECHECK: Duration = Duration::from_secs(2);
/// `$PATH` values whose scans are kept.
const PATH_SCANS: usize = 4;

/// One `$PATH` value's executables and the directory stamps they came from.
struct PathScan {
    dirs: Vec<(PathBuf, Stamp)>,
    /// No directory had changed within `RACY` of the scan.
    trusted: bool,
    checked: Instant,
    cmds: Arc<Vec<String>>,
}

static PATH_CACHE: LazyLock<Mutex<HashMap<OsString, PathScan>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The interactive shell's `$PATH` from its probe ([`crate::shell_env`]):
/// rc files extend it (`PNPM_HOME`, nvm).
static SHELL_PATH: Mutex<Option<OsString>> = Mutex::new(None);

/// Record the interactive shell's `$PATH`, which [`path_commands`] then
/// lists instead of Rift's own.
pub(crate) fn set_shell_path(path: &OsStr) {
    *SHELL_PATH.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.to_os_string());
}

/// Executable names on `$PATH` (sorted, deduped): the interactive shell's
/// `$PATH` reported by its probe ([`crate::shell_env::ShellEnv`]), else
/// Rift's own. See [`commands_on`] for when it is scanned again.
pub fn path_commands() -> Arc<Vec<String>> {
    let shell = SHELL_PATH
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    commands_on(&shell.unwrap_or_else(|| std::env::var_os("PATH").unwrap_or_default()))
}

/// Executable names in the directories of `path` (a `$PATH` value), sorted
/// and deduped. Cached per value; at most every `PATH_RECHECK` a request
/// compares the directories' stamps (one `stat` each) and scans again when
/// one changed (a command installed or removed).
pub fn commands_on(path: &OsStr) -> Arc<Vec<String>> {
    commands_on_at(path, Instant::now())
}

fn commands_on_at(path: &OsStr, now: Instant) -> Arc<Vec<String>> {
    let mut scans = PATH_CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(s) = scans.get_mut(path) {
        if now.saturating_duration_since(s.checked) < PATH_RECHECK {
            return s.cmds.clone();
        }
        s.checked = now;
        if s.trusted && s.dirs.iter().all(|(d, st)| Stamp::of(d) == *st) {
            return s.cmds.clone();
        }
    }
    let read_at = std::time::SystemTime::now();
    let cmds = Arc::new(scan_path_commands(path));
    let dirs: Vec<(PathBuf, Stamp)> = std::env::split_paths(path)
        .map(|d| {
            let s = Stamp::of(&d);
            (d, s)
        })
        .collect();
    let trusted = !dirs.iter().any(|(_, s)| s.racy(read_at));
    if !scans.contains_key(path)
        && scans.len() >= PATH_SCANS
        && let Some(old) = scans
            .iter()
            .min_by_key(|(_, s)| s.checked)
            .map(|(k, _)| k.clone())
    {
        scans.remove(&old);
    }
    scans.insert(
        path.to_os_string(),
        PathScan {
            dirs,
            trusted,
            checked: now,
            cmds: cmds.clone(),
        },
    );
    cmds
}

/// Executable names in the directories of `path` (a `$PATH` value), sorted
/// and deduped, read now (uncached; see [`commands_on`]).
pub fn scan_path_commands(path: &std::ffi::OsStr) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for dir in std::env::split_paths(path) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            // the mode bits need a stat; it follows symlinks (Homebrew's
            // bin is all links), so a link to a directory is not a command
            let is_exec = std::fs::metadata(e.path())
                .map(|m| {
                    use std::os::unix::fs::PermissionsExt;
                    !m.is_dir() && m.permissions().mode() & 0o111 != 0
                })
                .unwrap_or(false);
            if is_exec && let Some(n) = e.file_name().to_str() {
                names.push(n.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// How a candidate matches the typed word; lower sorts first (Warp's
/// completion matcher).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    /// Starts with it, ignoring ASCII case (the whole candidate too).
    Prefix,
    /// Holds its characters in order, ignoring case (`gco` →
    /// `git-checkout`); only for a typed word with a letter or digit
    /// (punctuation alone matches too much).
    Subsequence,
}

/// How `cand` matches `typed`, if at all ([`Match`]).
pub fn match_kind(cand: &str, typed: &str) -> Option<Match> {
    if prefix_case(cand, typed).is_some() {
        return Some(Match::Prefix);
    }
    if !typed.chars().any(char::is_alphanumeric) {
        return None;
    }
    let mut rest = cand.chars();
    typed
        .chars()
        .all(|tc| rest.any(|cc| same_char(cc, tc)))
        .then_some(Match::Subsequence)
}

/// Equal ignoring case.
fn same_char(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// `Some(case_sensitive)` when `cand` starts with `typed` ignoring ASCII
/// case: `true` when exactly as typed.
fn prefix_case(cand: &str, typed: &str) -> Option<bool> {
    if cand.starts_with(typed) {
        return Some(true);
    }
    let head = cand.get(..typed.len())?;
    head.eq_ignore_ascii_case(typed).then_some(false)
}

/// Indices of the `items` whose name (`None`: skipped) matches `typed`,
/// best first ([`Match`], then index), at most `cap`. Subsequence matches
/// are only tried when the prefix matches leave room, so a large list
/// costs one prefix check per item for a common prefix.
fn best_matches<T>(
    items: &[T],
    name: impl Fn(&T) -> Option<&str>,
    typed: &str,
    cap: usize,
) -> Vec<(usize, Match)> {
    let mut found: Vec<(usize, Match)> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| name(it).is_some_and(|n| prefix_case(n, typed).is_some()))
        .map(|(i, _)| (i, Match::Prefix))
        .take(cap)
        .collect();
    if found.len() < cap && typed.chars().any(char::is_alphanumeric) {
        let fuzzy = items.iter().enumerate().filter_map(|(i, it)| {
            let n = name(it)?;
            match match_kind(n, typed)? {
                Match::Prefix => None, // already taken
                m => Some((i, m)),
            }
        });
        let room = cap - found.len();
        found.extend(fuzzy.take(room));
    }
    found
}

/// How a menu row matches the word being completed (Warp's completion
/// menu): exactly, as its start, or fuzzily (a subsequence, with a score).
/// `cs`: exactly as typed (not only ignoring case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuMatch {
    Exact { cs: bool },
    Prefix { cs: bool },
    Fuzzy { score: i32 },
}

impl MenuMatch {
    /// An exact or prefix match exactly as typed: what Tab's single-match
    /// and common-prefix rules count.
    pub fn case_sensitive(self) -> bool {
        matches!(
            self,
            MenuMatch::Exact { cs: true } | MenuMatch::Prefix { cs: true }
        )
    }
}

/// How `cand` matches `query` for the menu, and the matched characters
/// (char indices, for highlighting): an exact or prefix match ignoring
/// ASCII case, else a subsequence when `query` has a letter or digit.
pub fn menu_match(cand: &str, query: &str) -> Option<(MenuMatch, Vec<usize>)> {
    if let Some(found) = menu_match_raw(cand, query) {
        return Some(found);
    }
    // a word typed with quotes or backslashes (`"./my`, `My\ Dir`) against
    // the backslash-escaped insert: both as literal text, the matched
    // characters mapped back onto the insert
    let typed = unescape_word(query);
    let mut literal = String::with_capacity(cand.len());
    let mut at = Vec::with_capacity(cand.len());
    let mut chars = cand.chars().enumerate();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            if let Some((j, n)) = chars.next() {
                literal.push(n);
                at.push(j);
            }
        } else {
            literal.push(c);
            at.push(i);
        }
    }
    if typed == query && literal == cand {
        return None;
    }
    menu_match_raw(&literal, &typed).map(|(kind, hits)| {
        (
            kind,
            hits.into_iter()
                .filter_map(|h| at.get(h).copied())
                .collect(),
        )
    })
}

fn menu_match_raw(cand: &str, query: &str) -> Option<(MenuMatch, Vec<usize>)> {
    if let Some(cs) = prefix_case(cand, query) {
        let n = query.chars().count();
        let kind = if cand.len() == query.len() {
            MenuMatch::Exact { cs }
        } else {
            MenuMatch::Prefix { cs }
        };
        return Some((kind, (0..n).collect()));
    }
    if !query.chars().any(char::is_alphanumeric) {
        return None;
    }
    fuzzy_score(cand, query).map(|(score, at)| (MenuMatch::Fuzzy { score }, at))
}

/// A subsequence match of `query` in `text`, ignoring case: the best
/// alignment's score and the matched char indices. Scored like fzf / skim:
/// each matched character counts, more at a word start (after `-_./:@`,
/// a space or a lower→upper change) and doubly so for the first one, a run
/// of consecutive characters gains, a gap costs.
pub fn fuzzy_score(text: &str, query: &str) -> Option<(i32, Vec<usize>)> {
    const HIT: i32 = 16;
    const BOUNDARY: i32 = 8;
    const CAMEL: i32 = 7;
    const RUN: i32 = 4;
    const GAP_START: i32 = -3;
    const GAP_MORE: i32 = -1;
    const NONE: i32 = i32::MIN / 2;
    let t: Vec<char> = text.chars().collect();
    let q: Vec<char> = query.chars().collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    // a cheap no first: most candidates are not subsequences
    let mut rest = t.iter();
    if q.len() > t.len() || !q.iter().all(|&qc| rest.any(|&tc| same_char(tc, qc))) {
        return None;
    }
    let bonus = |j: usize| -> i32 {
        let Some(&p) = j.checked_sub(1).and_then(|k| t.get(k)) else {
            return BOUNDARY;
        };
        let c = t[j];
        if !p.is_alphanumeric() {
            BOUNDARY
        } else if (p.is_lowercase() && c.is_uppercase()) || (p.is_alphabetic() && c.is_numeric()) {
            CAMEL
        } else {
            0
        }
    };
    let (n, m) = (t.len(), q.len());
    // score[i][j]: the best with q[..=i] matched and q[i] at t[j]; from:
    // where q[i - 1] sat; chunk: the bonus of the run's first character,
    // which the rest of a run keeps (fzf)
    let mut score = vec![vec![NONE; n]; m];
    let mut from = vec![vec![usize::MAX; n]; m];
    let mut chunk = vec![vec![0; n]; m];
    for i in 0..m {
        // the best earlier row entry two or more back, gap cost included
        let (mut gap_best, mut gap_at) = (NONE, usize::MAX);
        for j in i..n {
            if i > 0 && j >= 2 {
                if gap_best > NONE {
                    gap_best += GAP_MORE;
                }
                let c = score[i - 1][j - 2];
                if c > NONE && c + GAP_START > gap_best {
                    (gap_best, gap_at) = (c + GAP_START, j - 2);
                }
            }
            if !same_char(t[j], q[i]) {
                continue;
            }
            let b = bonus(j);
            if i == 0 {
                (score[0][j], chunk[0][j]) = (HIT + 2 * b, b);
                continue;
            }
            let (run, run_bonus) = match score[i - 1][j - 1] {
                s if s > NONE => {
                    let k = chunk[i - 1][j - 1].max(RUN);
                    (s + HIT + b.max(k), k)
                }
                _ => (NONE, 0),
            };
            let gap = if gap_best > NONE {
                gap_best + HIT + b
            } else {
                NONE
            };
            if run > NONE && run >= gap {
                (score[i][j], from[i][j], chunk[i][j]) = (run, j - 1, run_bonus);
            } else if gap > NONE {
                (score[i][j], from[i][j], chunk[i][j]) = (gap, gap_at, b);
            }
        }
    }
    let (end, &best) = score[m - 1]
        .iter()
        .enumerate()
        .filter(|(_, s)| **s > NONE)
        .max_by_key(|(j, s)| (**s, std::cmp::Reverse(*j)))?;
    let mut at = vec![end];
    let mut j = end;
    for i in (1..m).rev() {
        j = from[i][j];
        at.push(j);
    }
    at.reverse();
    Some((best, at))
}

/// The menu rows for `query` among `items` (Warp's order): the exact match
/// as typed, then one ignoring case, then the prefix matches in `items`'
/// order, then the subsequence matches by score; each with its matched
/// characters (char indices). Rows not matching are left out; a second
/// exact match is a duplicate and left out too.
pub fn filter_by_query(items: &[Completion], query: &str) -> Vec<(usize, MenuMatch, Vec<usize>)> {
    let mut exact: Option<(usize, MenuMatch, Vec<usize>)> = None;
    let mut exact_ci: Option<(usize, MenuMatch, Vec<usize>)> = None;
    let mut prefix = Vec::new();
    let mut fuzzy = Vec::new();
    for (i, c) in items.iter().enumerate() {
        let Some((kind, at)) = menu_match(&c.insert, query) else {
            continue;
        };
        match kind {
            MenuMatch::Exact { cs: true } if exact.is_none() => exact = Some((i, kind, at)),
            MenuMatch::Exact { cs: false } if exact_ci.is_none() => exact_ci = Some((i, kind, at)),
            MenuMatch::Prefix { .. } => prefix.push((i, kind, at)),
            MenuMatch::Fuzzy { .. } => fuzzy.push((i, kind, at)),
            _ => {}
        }
    }
    let score = |k: &MenuMatch| match k {
        MenuMatch::Fuzzy { score } => *score,
        _ => 0,
    };
    // best score first; equal scores keep their order
    fuzzy.sort_by_key(|(_, k, _)| std::cmp::Reverse(score(k)));
    exact
        .into_iter()
        .chain(exact_ci)
        .chain(prefix)
        .chain(fuzzy)
        .collect()
}

/// The longest common prefix of `words` (on character boundaries).
pub fn common_prefix<'a>(mut words: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let first = words.next()?;
    let mut len = first.len();
    for w in words {
        len = first
            .char_indices()
            .zip(w.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map_or(0, |((i, a), _)| i + a.len_utf8())
            .min(len);
    }
    Some(&first[..len])
}

/// What Tab does with the menu closed (Warp's explicit Tab completion),
/// for the candidates `items` of the word `query`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabAction {
    /// No candidate.
    Nothing,
    /// Exactly one prefix match as typed (or none such and exactly one
    /// ignoring case): insert `items[.0]`, no menu.
    Insert(usize),
    /// Insert the candidates' common prefix (longer than the word), then
    /// open the menu.
    CommonPrefix(String),
    /// Open the menu, the first row selected.
    Open,
}

/// [`TabAction`] for `items` (the word candidates, best first) of `query`.
pub fn tab_action(items: &[Completion], query: &str) -> TabAction {
    if filter_by_query(items, query).is_empty() {
        return TabAction::Nothing;
    }
    let kinds: Vec<Option<MenuMatch>> = items
        .iter()
        .map(|c| menu_match(&c.insert, query).map(|(k, _)| k))
        .collect();
    let cs: Vec<usize> = (0..items.len())
        .filter(|&i| kinds[i].is_some_and(MenuMatch::case_sensitive))
        .collect();
    let ci: Vec<usize> = (0..items.len())
        .filter(|&i| {
            matches!(
                kinds[i],
                Some(MenuMatch::Exact { cs: false } | MenuMatch::Prefix { cs: false })
            )
        })
        .collect();
    match (cs.as_slice(), ci.as_slice()) {
        ([one], _) | ([], [one]) => return TabAction::Insert(*one),
        _ => {}
    }
    if let Some(p) = common_prefix(cs.iter().map(|&i| items[i].insert.as_str()))
        && p.len() > query.len()
        && p.starts_with(query)
    {
        return TabAction::CommonPrefix(p.to_string());
    }
    TabAction::Open
}

/// What Tab does in the open menu: the common prefix of the rows matching
/// as typed (`rows`, with their match), when longer than the word `query`
/// (`close`: it was the only such row), else `None` (the next row).
pub fn menu_tab(rows: &[(&str, MenuMatch)], query: &str) -> Option<(String, bool)> {
    let cs: Vec<&str> = rows
        .iter()
        .filter(|(_, k)| k.case_sensitive())
        .map(|(s, _)| *s)
        .collect();
    let p = common_prefix(cs.iter().copied())?;
    (p.len() > query.len() && p.starts_with(query)).then(|| (p.to_string(), cs.len() == 1))
}

/// Whether an accepted completion `result` gets a space after it (Warp):
/// only when it ends the command line (`at_end`), does not end in `/` (a
/// path goes on) or `=` (a value follows), and the menu does not open as
/// you type.
pub fn space_after(result: &str, at_end: bool, as_you_type: bool) -> bool {
    at_end && !as_you_type && !result.ends_with('/') && !result.ends_with('=')
}

/// Whether ⏎ accepts the selected row (`selected`) instead of running the
/// line: always with the menu opened by Tab; as you type, unless what
/// stands from the word's start to the end of the line (`typed`) already
/// is it (a trailing `/` aside).
pub fn enter_accepts(as_you_type: bool, typed: &str, selected: Option<&str>) -> bool {
    if !as_you_type {
        return selected.is_some();
    }
    selected.is_none_or(|s| typed != s && s.strip_suffix('/').is_none_or(|bare| typed != bare))
}

/// Order of the sources at equal [`Match`]: command names, subcommands,
/// options, generator values, paths.
fn source_rank(source: &str) -> u8 {
    match source {
        "command" => 0,
        "shell" => 1,
        "option" => 2,
        "path" => 4,
        _ => 3,
    }
}

/// Word candidates kept; the rest of the 50 are history lines.
const MAX_WORDS: usize = 45;
/// History lines kept (prefix matches of the whole buffer).
const MAX_HISTORY: usize = 5;
/// History entries read for recency ranking, newest first: the pane's own
/// and the imported shell history's.
const RECENCY_PANE: usize = 200;
const RECENCY_SHELL: usize = 500;

/// For each word of the recent history, how recently it was used (0 =
/// the newest line). A trailing `/` is ignored, so `src` and `src/` agree.
fn recency<'a>(history: &'a [String], shell_history: &'a [String]) -> HashMap<&'a str, usize> {
    let mut map = HashMap::new();
    let lines = history
        .iter()
        .rev()
        .take(RECENCY_PANE)
        .chain(shell_history.iter().rev().take(RECENCY_SHELL));
    for (age, line) in lines.enumerate() {
        for w in line.split_whitespace() {
            map.entry(w.trim_end_matches('/')).or_insert(age);
        }
    }
    map
}

/// Sort word candidates: match kind (a subsequence by its score against
/// `typed`, best first), source, recency, then each provider's own order
/// (the sort is stable).
fn rank(
    words: &mut [(Completion, Match)],
    typed: &str,
    history: &[String],
    shell_history: &[String],
) {
    if words.len() < 2 {
        return;
    }
    let recent = recency(history, shell_history);
    let age = |c: &Completion| {
        recent
            .get(c.insert.trim_end_matches('/'))
            .copied()
            .unwrap_or(usize::MAX)
    };
    let score = |c: &Completion, m: Match| match m {
        Match::Prefix => 0,
        Match::Subsequence => fuzzy_score(&c.insert, typed).map_or(0, |(s, _)| s),
    };
    words.sort_by_cached_key(|(c, m)| {
        (
            *m,
            std::cmp::Reverse(score(c, *m)),
            source_rank(c.source),
            age(c),
        )
    });
}

/// Gather completions for the token at `cursor`. `history` is the pane's
/// own (newest last); `shell_history` the imported shell history; `cwd`
/// the pane's local directory (`None`: none on this machine, so no path
/// or file-based argument completions).
///
/// Word candidates are ranked by [`Match`] (prefix, then word start, then
/// subsequence), then by source (command names, subcommands, options,
/// generator values, paths), then by how recently the
/// word appears in the history, then in each provider's own order; at most
/// 45. History lines (whole-buffer prefix matches, newest first, at most 5)
/// follow them.
pub fn complete(
    buffer: &str,
    cursor: usize,
    history: &[String],
    shell_history: &[String],
    cwd: Option<&str>,
) -> Vec<Completion> {
    complete_on_path(buffer, cursor, history, shell_history, cwd, None)
}

/// [`complete`] with command names from `path` (the pane shell's `$PATH`,
/// e.g. after `rift-use`) instead of [`path_commands`] when given.
pub fn complete_on_path(
    buffer: &str,
    cursor: usize,
    history: &[String],
    shell_history: &[String],
    cwd: Option<&str>,
    path: Option<&OsStr>,
) -> Vec<Completion> {
    gather(buffer, cursor, cwd, path).finish(buffer, history, shell_history)
}

/// The word candidates for one request before ranking: Rift's own
/// providers ([`gather`]), then other sources merged in ([`Gathered::add`]),
/// then [`Gathered::finish`].
#[derive(Debug, Clone)]
pub struct Gathered {
    words: Vec<(Completion, Match)>,
    /// The word being completed, unescaped.
    typed: String,
    /// The word being completed, as typed.
    token: String,
    /// It is the command name of its command.
    first_word: bool,
    /// What a spec says the word is (`ArgKind::None`: no spec or not an
    /// argument).
    arg: ArgKind,
}

/// Rift's own word candidates for the token at `cursor` (see
/// [`complete_on_path`]).
pub fn gather(buffer: &str, cursor: usize, cwd: Option<&str>, path: Option<&OsStr>) -> Gathered {
    gather_with(buffer, cursor, cwd, Some(path))
}

/// [`gather`] for a pane on another machine: nothing from this one (no
/// directory, no `$PATH` commands); the remote answer is merged in
/// ([`crate::remote_complete::complete_into`]).
pub fn gather_remote(buffer: &str, cursor: usize) -> Gathered {
    gather_with(buffer, cursor, None, None)
}

/// [`gather`]; `path: None` lists no command names at all.
fn gather_with(
    buffer: &str,
    cursor: usize,
    cwd: Option<&str>,
    path: Option<Option<&OsStr>>,
) -> Gathered {
    let (token_start, token) = current_token(buffer, cursor);
    let mut words: Vec<(Completion, Match)> = Vec::new();
    // a spec path/dir argument is completed by its generator alone
    let mut generic_paths = true;
    let mut arg = ArgKind::None;

    // Which command are we completing inside? A pipe / `;` / `&&` / `||`
    // starts a fresh command, so only look at the current segment.
    let seg_start = completion_specs::segment_start(&buffer[..token_start]);
    let seg_prefix = &buffer[seg_start..token_start];
    let first_word = seg_prefix.trim().is_empty();

    // command names: first word of the segment, from $PATH (Warp-style).
    // Names are filenames: insert them escaped so `;`, quotes or `$` in a
    // basename stay one word; names with control characters are skipped.
    if first_word && let Some(path) = path {
        let cmds = path.map_or_else(path_commands, commands_on);
        words.extend(command_completions(&cmds, token));
    }

    // spec-driven: subcommands, options, and argument generators
    if !first_word {
        let args = completion_specs::tokenize(seg_prefix);
        if let Some((cmd, args)) = args.split_first()
            && let Some(spec) = completion_specs::lookup(cmd)
        {
            let res = completion_specs::resolve(spec, args);
            if token.starts_with('-') {
                words.extend(option_completions(&res, token));
            } else {
                if res.offer_subcommands {
                    for s in res.subcommands {
                        if let Some(m) = match_kind(s.name, token) {
                            words.push((
                                Completion {
                                    insert: s.name.into(),
                                    description: s.desc.into(),
                                    source: "shell",
                                },
                                m,
                            ));
                        }
                    }
                }
                generic_paths = !matches!(res.args, ArgKind::Path | ArgKind::Dir);
                arg = res.args;
                words.extend(generator_completions(res.args, token, cwd));
            }
        }
    }

    // paths: token that looks like one
    let word = unescape_word(token);
    if generic_paths && (word.contains('/') || word.starts_with('.') || word.starts_with('~')) {
        words.extend(complete_path(&word, cwd, false));
    }
    Gathered {
        words,
        typed: word,
        token: token.to_string(),
        first_word,
        arg,
    }
}

impl Gathered {
    /// Merge literal words from another source (`source`, e.g. "shell" for
    /// the user's shell's own completions): each is escaped as one word; a
    /// word Rift already offers keeps Rift's entry (a directory with or
    /// without its trailing `/`: bash's `complete -d` lists `src`, Rift
    /// `src/`), and one with a control character is skipped.
    pub fn add(&mut self, items: &[(String, String)], source: &'static str) {
        let have: std::collections::HashSet<String> = self
            .words
            .iter()
            .map(|(c, _)| c.insert.trim_end_matches('/').to_string())
            .collect();
        let mut added = std::collections::HashSet::new();
        for (value, desc) in items {
            if value.is_empty() {
                continue;
            }
            let Some(insert) = escape_word(value) else {
                continue;
            };
            let key = insert.trim_end_matches('/').to_string();
            if have.contains(&key) || !added.insert(key) {
                continue;
            }
            // the source matched it already; keep its match, ranked last
            let m = match_kind(value, &self.typed).unwrap_or(Match::Subsequence);
            self.words.push((
                Completion {
                    insert,
                    description: desc.chars().filter(|c| !c.is_control()).collect(),
                    source,
                },
                m,
            ));
        }
    }

    /// Merge values listed elsewhere (a remote machine's git refs) that
    /// match the typed word, best first, at most `cap`; `source_of` maps a
    /// value's description to its source.
    pub fn add_matching(
        &mut self,
        items: &[(String, String)],
        source_of: impl Fn(&str) -> &'static str,
        cap: usize,
    ) {
        let found = best_matches(
            items,
            |(v, _)| (!v.chars().any(char::is_control)).then_some(v.as_str()),
            &self.typed,
            cap,
        );
        for (i, m) in found {
            let (v, desc) = &items[i];
            if let Some(insert) = escape_word(v) {
                self.words.push((
                    Completion {
                        insert,
                        description: desc.clone(),
                        source: source_of(desc),
                    },
                    m,
                ));
            }
        }
    }

    /// Merge command names listed elsewhere (a remote `$PATH`), as the
    /// first word of a command (see [`complete_on_path`]).
    pub fn add_commands(&mut self, cmds: &[String]) {
        if !self.first_word {
            return;
        }
        let mut sorted = cmds.to_vec();
        sorted.sort();
        sorted.dedup();
        self.words.extend(command_completions(&sorted, &self.token));
    }

    /// Merge the entries of a directory listed elsewhere (a remote one;
    /// directories end in `/`): the typed word's directory part, as
    /// [`complete_on_path`] completes local paths.
    pub fn add_paths(&mut self, names: &[String]) {
        let typed = self.typed.as_str();
        let (dir, file) = match typed.rfind('/') {
            Some(i) => typed.split_at(i + 1),
            None => ("", typed),
        };
        let hidden = file.starts_with('.');
        let found = best_matches(
            names,
            |n| {
                ((hidden || !n.starts_with('.')) && !n.chars().any(char::is_control))
                    .then_some(n.as_str())
            },
            file,
            MAX_PATHS,
        );
        for (i, m) in found {
            let name = &names[i];
            if let Some(insert) = escape_word(&format!("{dir}{name}")) {
                self.words.push((
                    Completion {
                        insert,
                        description: if name.ends_with('/') {
                            "directory".into()
                        } else {
                            String::new()
                        },
                        source: "path",
                    },
                    m,
                ));
            }
        }
    }

    /// The word being completed, unescaped.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Whether the word being completed is a command name.
    pub fn first_word(&self) -> bool {
        self.first_word
    }

    /// What a spec says the word being completed is.
    pub fn arg_kind(&self) -> ArgKind {
        self.arg
    }

    /// Whether the word being completed looks like a path (`/`, `.`, `~`).
    pub fn path_like(&self) -> bool {
        let w = &self.typed;
        w.contains('/') || w.starts_with('.') || w.starts_with('~')
    }

    /// Rank the words and append the history lines (see [`complete`]).
    pub fn finish(
        self,
        buffer: &str,
        history: &[String],
        shell_history: &[String],
    ) -> Vec<Completion> {
        self.finish_ranked(buffer, &[], history, shell_history)
    }

    /// [`finish`](Self::finish) with `ranked` history lines first: matches
    /// of Rift's own history ranked by directory
    /// (`command_history::ranked_matches`), best first.
    pub fn finish_ranked(
        self,
        buffer: &str,
        ranked: &[&str],
        history: &[String],
        shell_history: &[String],
    ) -> Vec<Completion> {
        self.finish_with(buffer, ranked, history, shell_history, &mut |_| true)
    }

    /// [`finish_ranked`](Self::finish_ranked), leaving out the history lines
    /// whose spec path arguments do not exist in `cwd` (the pane's local
    /// directory; `None`: no check), [`command_valid`]. The checks share
    /// [`VALIDATE_BUDGET`]; lines past it are kept unchecked.
    pub fn finish_checked(
        self,
        buffer: &str,
        ranked: &[&str],
        history: &[String],
        shell_history: &[String],
        cwd: Option<&str>,
    ) -> Vec<Completion> {
        self.finish_checked_except(buffer, ranked, history, shell_history, cwd, &|_| false)
    }

    /// [`finish_checked`](Self::finish_checked), leaving out too the
    /// history lines `ignored` names (`ignored_suggestions`: the ghost
    /// text's ✕), so they hold no place of a line to suggest.
    pub fn finish_checked_except(
        self,
        buffer: &str,
        ranked: &[&str],
        history: &[String],
        shell_history: &[String],
        cwd: Option<&str>,
        ignored: &dyn Fn(&str) -> bool,
    ) -> Vec<Completion> {
        let deadline = Instant::now() + VALIDATE_BUDGET;
        self.finish_checked_until(
            buffer,
            ranked,
            history,
            shell_history,
            cwd,
            deadline,
            ignored,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_checked_until(
        self,
        buffer: &str,
        ranked: &[&str],
        history: &[String],
        shell_history: &[String],
        cwd: Option<&str>,
        deadline: Instant,
        ignored: &dyn Fn(&str) -> bool,
    ) -> Vec<Completion> {
        self.finish_with(buffer, ranked, history, shell_history, &mut |line| {
            !ignored(line)
                && cwd.is_none_or(|cwd| Instant::now() >= deadline || command_valid(line, cwd))
        })
    }

    fn finish_with(
        self,
        buffer: &str,
        ranked: &[&str],
        history: &[String],
        shell_history: &[String],
        valid: &mut dyn FnMut(&str) -> bool,
    ) -> Vec<Completion> {
        let mut words = self.words;
        rank(&mut words, &self.token, history, shell_history);
        let mut seen = std::collections::HashSet::new();
        let out: Vec<Completion> = words
            .into_iter()
            .map(|(c, _)| c)
            .filter(|c| seen.insert(c.insert.clone()))
            .take(MAX_WORDS)
            .collect();
        with_history(out, seen, buffer, ranked, history, shell_history, valid)
    }
}

/// The time [`Gathered::finish_checked`] spends checking history lines.
pub const VALIDATE_BUDGET: Duration = Duration::from_millis(150);

/// How many words of a command [`command_valid`] reads.
const VALIDATE_WORDS: usize = 64;

/// The commands with a Rift spec whose path argument must already exist.
/// Others with a path spec are left out: `grep`/`rg` take a pattern first,
/// `tar` a mode, `scp` a destination, `git add`/`rm`/`restore` name deleted
/// files, `cargo new` a directory to create.
const READS_EXISTING: &[&str] = &["cat", "cd", "ls", "python", "python3"];

/// Script runners: their first argument that is not an option is the
/// script they read, which must exist.
const SCRIPT_RUNNERS: &[&str] = &[
    "bash", "sh", "zsh", "fish", "node", "ruby", "perl", "source", ".",
];

/// Whether `command` may be suggested in directory `cwd` (Warp's
/// autosuggestion validation): in each of its commands whose spec path
/// argument must exist (`READS_EXISTING`), the first positional argument
/// must name a path there (a directory for `cd`), and a script runner's
/// (`SCRIPT_RUNNERS`) script must be a file there. Any other command, an
/// option the spec does not know (a parse it cannot follow), code given
/// inline (`bash -c`, `node -e`), a word with a variable, glob or `~user`,
/// a remote `host:path` or a redirection is taken as valid.
pub fn command_valid(command: &str, cwd: &str) -> bool {
    let mut end = command.len();
    loop {
        let start = completion_specs::segment_start(&command[..end]);
        if !segment_valid(&command[start..end], cwd) {
            return false;
        }
        if start == 0 {
            return true;
        }
        // drop the separator (`|`, `||`, `&&`, `;`, a newline) before it
        end = command[..start]
            .trim_end_matches(['|', '&', ';', '\n', ' ', '\t'])
            .len();
        if end == 0 {
            return true;
        }
    }
}

fn segment_valid(segment: &str, cwd: &str) -> bool {
    let words = completion_specs::tokenize(segment);
    let Some((cmd, args)) = words.split_first() else {
        return true;
    };
    if SCRIPT_RUNNERS.contains(&cmd.as_str()) {
        return script_valid(cmd, &args[..args.len().min(VALIDATE_WORDS)], cwd);
    }
    let Some(spec) =
        completion_specs::lookup(cmd).filter(|_| READS_EXISTING.contains(&cmd.as_str()))
    else {
        return true;
    };
    let args = &args[..args.len().min(VALIDATE_WORDS)];
    let mut i = 0;
    while i < args.len() {
        let w = &args[i];
        let res = completion_specs::resolve(spec, &args[..i]);
        if w.trim_start_matches(|c: char| c.is_ascii_digit())
            .starts_with(['<', '>', '&'])
        {
            // a redirection: its target is not an argument
            return true;
        }
        if w.starts_with('-') && w != "-" && w != "--" {
            let Some(o) = completion_specs::find_option(&res.options, w) else {
                return true;
            };
            i += if o.takes_arg && !w.contains('=') {
                2
            } else {
                1
            };
            continue;
        }
        if w == "-" || w == "--" {
            i += 1;
            continue;
        }
        if res.offer_subcommands && res.subcommands.iter().any(|s| s.name == *w) {
            i += 1;
            continue;
        }
        // the first positional at this level decides
        return match res.args {
            ArgKind::Path => path_exists(w, cwd, false),
            ArgKind::Dir => path_exists(w, cwd, true),
            _ => true,
        };
    }
    true
}

/// A script runner `cmd`'s arguments `args`: the first one that is not an
/// option (nor an option's value) must be a file in `cwd`. Code given
/// inline or read from stdin (`bash -c`, `sh -s`, `node -e`, `-`), a help
/// or version flag, or no script at all is valid.
fn script_valid(cmd: &str, args: &[String], cwd: &str) -> bool {
    let shell = matches!(cmd, "bash" | "sh" | "zsh");
    // options whose value is the next word, and those giving inline code
    let (takes_value, inline): (&[&str], &[&str]) = match cmd {
        "bash" | "sh" | "zsh" => (&["-o", "+o", "-O", "+O", "--rcfile", "--init-file"], &[]),
        "fish" => (
            &[
                "-C",
                "--init-command",
                "-d",
                "--debug",
                "-o",
                "--debug-output",
                "-f",
                "--features",
            ],
            &["-c", "--command"],
        ),
        "node" => (
            &[
                "-r",
                "--require",
                "--import",
                "--loader",
                "--experimental-loader",
                "-C",
                "--conditions",
                "--input-type",
                "--title",
                "--env-file",
            ],
            &[
                "-e",
                "--eval",
                "-p",
                "--print",
                "-i",
                "--interactive",
                "--test",
            ],
        ),
        "ruby" => (&["-I", "-r", "-C", "-E", "-F"], &["-e"]),
        "perl" => (&["-I", "-M", "-m"], &["-e", "-E"]),
        _ => (&[], &[]),
    };
    let mut i = 0;
    while i < args.len() {
        let w = args[i].as_str();
        if w.trim_start_matches(|c: char| c.is_ascii_digit())
            .starts_with(['<', '>', '&'])
        {
            // a redirection (`bash < script`): no script argument
            return true;
        }
        let version = matches!(w, "-h" | "-v" | "-V") && !shell;
        if w == "-" || version || matches!(w, "--help" | "--version") {
            return true;
        }
        if w == "--" {
            return args.get(i + 1).is_none_or(|s| script_exists(s, cwd));
        }
        let option = w.starts_with('-') || (shell && w.starts_with('+'));
        if !option {
            return script_exists(w, cwd);
        }
        if inline.contains(&w)
            || inline
                .iter()
                .any(|o| o.starts_with("--") && w.starts_with(&format!("{o}=")))
        {
            return true;
        }
        // a shell's short options together (`-xc`): `c` or `s` read no file
        if shell && !w.starts_with("--") && w[1..].contains(['c', 's']) {
            return true;
        }
        i += if takes_value.contains(&w) { 2 } else { 1 };
    }
    true
}

/// Whether the path argument `word` names something in `cwd` (a directory
/// when `dir`); a word Rift cannot resolve without the shell counts.
fn path_exists(word: &str, cwd: &str, dir: bool) -> bool {
    match resolve(word, cwd) {
        Some(path) if dir => path.is_dir(),
        Some(path) => path.exists(),
        None => true,
    }
}

/// Whether the script argument `word` is a file in `cwd` (see
/// [`path_exists`]).
fn script_exists(word: &str, cwd: &str) -> bool {
    resolve(word, cwd).is_none_or(|p| p.is_file())
}

/// The path a path argument names in `cwd`; `None` for a word Rift cannot
/// resolve without the shell (a variable, glob, `~user`, `host:path`).
fn resolve(word: &str, cwd: &str) -> Option<PathBuf> {
    if word.is_empty()
        || word.contains(['$', '*', '?', '[', '{', '`', ':', '<', '>', '&'])
        || (word.starts_with('~') && word != "~" && !word.starts_with("~/"))
    {
        return None;
    }
    Some(match word.strip_prefix('~') {
        Some(rest) => PathBuf::from(std::env::var_os("HOME")?).join(rest.trim_start_matches('/')),
        None => PathBuf::from(cwd).join(word),
    })
}

/// `out` followed by the history lines for `buffer` that are `valid`.
fn with_history(
    mut out: Vec<Completion>,
    mut seen: std::collections::HashSet<String>,
    buffer: &str,
    ranked: &[&str],
    history: &[String],
    shell_history: &[String],
    valid: &mut dyn FnMut(&str) -> bool,
) -> Vec<Completion> {
    // history: whole-buffer prefix matches (drives ghost text); Rift's own
    // history ranked by directory first, then the pane's own entries, then
    // the imported shell history (newest first)
    if !buffer.trim().is_empty() {
        let mut n = 0;
        for h in ranked.iter().copied().chain(
            history
                .iter()
                .rev()
                .take(200)
                .chain(shell_history.iter().rev().take(3000))
                .map(String::as_str),
        ) {
            if h.starts_with(buffer)
                && h != buffer
                && !h.contains('\n')
                // a line left out is not checked again
                && seen.insert(h.to_string())
                && valid(h)
            {
                out.push(Completion {
                    insert: h.to_string(),
                    description: String::new(),
                    source: "history",
                });
                n += 1;
                if n >= MAX_HISTORY {
                    break;
                }
            }
        }
    }
    out
}

/// Command-name completions for `token` (as typed) from the sorted `cmds`,
/// at most 12: the names starting with it as typed, then those starting
/// with it in another case, then those holding it as a subsequence (see
/// [`match_kind`]). An empty word (nothing typed yet) lists none. Inserts
/// are [`escape_word`]-escaped.
fn command_completions(cmds: &[String], token: &str) -> Vec<(Completion, Match)> {
    const MAX: usize = 12;
    let typed = unescape_word(token);
    if typed.is_empty() || typed.contains('/') || typed.starts_with('.') || typed.starts_with('~') {
        return Vec::new();
    }
    let start = cmds.partition_point(|c| c.as_str() < typed.as_str());
    let mut found: Vec<(&String, Match)> = cmds[start..]
        .iter()
        .take_while(|c| c.starts_with(&typed))
        .map(|c| (c, Match::Prefix))
        .take(MAX)
        .collect();
    if found.len() < MAX {
        let mut other: Vec<(&String, Match)> = cmds
            .iter()
            .filter(|c| !c.starts_with(&typed))
            .filter_map(|c| Some((c, match_kind(c, &typed)?)))
            .collect();
        // the other case's prefixes first (stable: name order within)
        other.sort_by_key(|(_, m)| *m);
        found.extend(other.into_iter().take(MAX - found.len()));
    }
    found
        .into_iter()
        .filter_map(|(name, m)| {
            Some((
                Completion {
                    insert: escape_command(name)?,
                    description: String::new(),
                    source: "command",
                },
                m,
            ))
        })
        .collect()
}

/// Option / flag completions for a token beginning with `-`.
fn option_completions(res: &completion_specs::Resolved, token: &str) -> Vec<(Completion, Match)> {
    // `--opt=va…`: the value is not an option name, and an option insert
    // would replace the whole token and drop `=va…`
    if token.contains('=') {
        return Vec::new();
    }
    let mut out = Vec::new();
    for o in &res.options {
        // Choose the form to offer: bare "-" lists every option (long form
        // preferred); "--…" matches long forms (past the dashes, also by
        // word start or subsequence); "-x" matches short forms by prefix.
        let found: Option<(&str, Match)> = if token == "-" {
            if !o.long.is_empty() {
                Some((o.long, Match::Prefix))
            } else if !o.short.is_empty() {
                Some((o.short, Match::Prefix))
            } else {
                None
            }
        } else if let Some(key) = token.strip_prefix("--") {
            o.long
                .strip_prefix("--")
                .and_then(|long| match_kind(long, key))
                .map(|m| (o.long, m))
        } else {
            (!o.short.is_empty() && o.short.starts_with(token)).then_some((o.short, Match::Prefix))
        };
        let Some((form, m)) = found else { continue };
        // skip a non-repeatable flag already present in the command
        if !o.repeatable
            && res.used_options.iter().any(|u| {
                let uk = u.split('=').next().unwrap_or(u);
                (!o.short.is_empty() && uk == o.short) || (!o.long.is_empty() && uk == o.long)
            })
        {
            continue;
        }
        out.push((
            Completion {
                insert: form.to_string(),
                description: o.desc.to_string(),
                source: "option",
            },
            m,
        ));
    }
    out
}

/// Values produced by a spec argument generator that match `token`, best
/// matches first, at most 50 each. `cwd: None` (no local directory): only
/// ssh hosts, which come from the home directory.
fn generator_completions(
    kind: ArgKind,
    token: &str,
    cwd: Option<&str>,
) -> Vec<(Completion, Match)> {
    // generator values are literal text: match against the unescaped token
    // and insert escaped, so a ref named `x$(cmd)` cannot run on Enter
    let want = unescape_word(token);
    // every generator but ssh hosts (the user's own ~/.ssh) reads the
    // shell's directory: none known locally, none to read
    if cwd.is_none() && !matches!(kind, ArgKind::SshHost) {
        return Vec::new();
    }
    // git refs carry a per-ref kind ("branch"/"remote"/"tag") in the desc
    let ref_source = |desc: &str| -> &'static str {
        match desc {
            "branch" => "branch",
            "tag" => "tag",
            _ => "remote-branch",
        }
    };
    let ranked = |items: &[(String, String)],
                  keep: &dyn Fn(&str) -> bool,
                  src: &dyn Fn(&str) -> &'static str|
     -> Vec<(Completion, Match)> {
        // a value with control characters cannot be inserted (escape_word):
        // skipped before the cap
        best_matches(
            items,
            |(v, d)| (keep(d) && !v.chars().any(char::is_control)).then_some(v.as_str()),
            &want,
            50,
        )
        .into_iter()
        .filter_map(|(i, m)| {
            let (v, desc) = &items[i];
            Some((
                Completion {
                    insert: escape_word(v)?,
                    description: desc.clone(),
                    source: src(desc),
                },
                m,
            ))
        })
        .take(50)
        .collect()
    };
    let all = |_: &str| true;
    match kind {
        ArgKind::None => Vec::new(),
        ArgKind::Path => complete_path(&want, cwd, false),
        ArgKind::Dir => complete_path(&want, cwd, true),
        ArgKind::GitRef => ranked(&completion_specs::git_refs_all(cwd), &all, &ref_source),
        ArgKind::GitBranch => ranked(
            &completion_specs::git_refs_all(cwd),
            &|d| d == "branch",
            &ref_source,
        ),
        ArgKind::GitRemote => ranked(&completion_specs::git_remotes(cwd), &all, &|_| "remote"),
        ArgKind::SshHost => ranked(&completion_specs::ssh_hosts_all(), &all, &|_| "ssh-host"),
        ArgKind::MakeTarget => ranked(
            &completion_specs::make_targets_all(cwd),
            &all,
            &|_| "make-target",
        ),
        ArgKind::PkgScript => ranked(
            &completion_specs::package_scripts_all(cwd),
            &all,
            &|_| "script",
        ),
    }
}

/// Path entries kept per request.
const MAX_PATHS: usize = 30;

/// Entries matching the literal (unescaped) path `token` (directories only
/// with `dirs_only`), best matches first, then by name; each `insert` is
/// escaped as one shell word. Listings come from the directory cache
/// ([`crate::completion_cache::dir_items`]). `cwd: None` (no local
/// directory): nothing, not even absolute or `~/` paths.
fn complete_path(token: &str, cwd: Option<&str>, dirs_only: bool) -> Vec<(Completion, Match)> {
    let Some(cwd) = cwd else {
        return Vec::new();
    };
    // Expand only "~/…": expansion must not change the basename, or the
    // orig_dir arithmetic below breaks (bare "~" expanding to $HOME made
    // file_part longer than the token itself → slice-index underflow panic).
    let expanded = if let Some(rest) = token.strip_prefix("~/") {
        match std::env::var("HOME") {
            Ok(home) => format!("{home}/{rest}"),
            Err(_) => token.to_string(),
        }
    } else {
        token.to_string()
    };

    let (dir_part, file_part) = match expanded.rfind('/') {
        Some(i) => (&expanded[..=i], &expanded[i + 1..]),
        None => ("", expanded.as_str()),
    };

    let base = if dir_part.starts_with('/') {
        PathBuf::from(dir_part)
    } else {
        PathBuf::from(cwd).join(dir_part)
    };

    let items = crate::completion_cache::dir_items(&base);
    // hidden entries only when the typed name starts with `.`
    let hidden = file_part.starts_with('.');
    // listings are sorted by name: index order is name order. A name
    // with control characters cannot be inserted (escape_word): skipped
    // before the cap.
    let found = best_matches(
        &items,
        |it| {
            ((hidden || !it.name.starts_with('.'))
                && (!dirs_only || it.is_dir)
                && !it.name.chars().any(char::is_control))
            .then_some(it.name.as_str())
        },
        file_part,
        MAX_PATHS,
    );
    // keep the user's original dir prefix (incl. ~) in the insert
    let orig_dir = &token[..token.len().saturating_sub(file_part.len())];
    found
        .into_iter()
        .filter_map(|(i, m)| {
            let it = &items[i];
            Some((
                Completion {
                    insert: escape_word(&format!(
                        "{orig_dir}{}{}",
                        it.name,
                        if it.is_dir { "/" } else { "" }
                    ))?,
                    description: if it.is_dir {
                        "directory".into()
                    } else {
                        String::new()
                    },
                    source: "path",
                },
                m,
            ))
        })
        .take(MAX_PATHS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_subcommands() {
        let out = complete("git ch", 6, &[], &[], None);
        let names: Vec<&str> = out.iter().map(|c| c.insert.as_str()).collect();
        assert!(names.contains(&"checkout"));
        assert!(names.contains(&"cherry-pick"));
        assert!(names.contains(&"check-ignore"));
        assert!(!names.contains(&"commit"));
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn shell_words_merge_and_rift_specs_win_duplicates() {
        let mut g = gather("git ch", 6, None, None);
        g.add(
            &pairs(&[
                ("checkout", "from the shell"),
                ("chlorine", "only the shell has it"),
                ("my file", ""),
                ("bad\x07", ""),
                ("ch", "the typed word itself"),
            ]),
            "shell",
        );
        let out = g.finish("git ch", &[], &[]);
        let checkout: Vec<&Completion> = out.iter().filter(|c| c.insert == "checkout").collect();
        assert_eq!(checkout.len(), 1, "one row per word");
        assert_ne!(
            checkout[0].description, "from the shell",
            "Rift's spec row wins"
        );
        let names: Vec<&str> = out.iter().map(|c| c.insert.as_str()).collect();
        assert!(names.contains(&"chlorine"), "{names:?}");
        assert!(names.contains(&"my\\ file"), "inserted escaped: {names:?}");
        assert!(!names.iter().any(|n| n.contains('\x07')));
        // the typed word itself is a row too (Warp's exact match)
        assert!(names.contains(&"ch"));
        // control: merged in first, the shell's row would be the one kept
        let mut g = gather("git ch", 6, None, None);
        let rift_row = g
            .words
            .iter()
            .position(|(c, _)| c.insert == "checkout")
            .unwrap();
        g.words.remove(rift_row);
        g.add(&pairs(&[("checkout", "from the shell")]), "shell");
        let out = g.finish("git ch", &[], &[]);
        assert!(out.iter().any(|c| c.description == "from the shell"));
    }

    #[test]
    fn listed_paths_commands_and_refs_merge_like_local_ones() {
        // a pane with no local directory: the listing comes from elsewhere
        let mut g = gather("cat src/ma", 10, None, None);
        assert!(g.path_like() && !g.first_word());
        g.add_paths(&[
            "main.rs".into(),
            "map/".into(),
            ".hidden".into(),
            "lib.rs".into(),
        ]);
        let out = g.finish("cat src/ma", &[], &[]);
        let names: Vec<&str> = out.iter().map(|c| c.insert.as_str()).collect();
        assert_eq!(names, vec!["src/main.rs", "src/map/"]);
        let mut g = gather("cat .h", 6, None, None);
        g.add_paths(&[".hidden".into(), "x".into()]);
        assert_eq!(g.finish("cat .h", &[], &[])[0].insert, ".hidden");

        let mut g = gather("dpk", 3, None, Some(OsStr::new("")));
        g.add_commands(&["dpkg-query".into(), "dpkg".into(), "ls".into()]);
        let names: Vec<String> = g
            .finish("dpk", &[], &[])
            .into_iter()
            .map(|c| c.insert)
            .collect();
        assert_eq!(names, vec!["dpkg", "dpkg-query"]);
        // command names only in command position
        let mut g = gather("cat dpk", 7, None, None);
        g.add_commands(&["dpkg".into()]);
        assert!(g.finish("cat dpk", &[], &[]).is_empty());

        let mut g = gather("git checkout fea", 16, None, None);
        assert_eq!(g.arg_kind(), ArgKind::GitRef);
        g.add_matching(
            &pairs(&[("feature-x", "branch"), ("main", "branch"), ("v1", "tag")]),
            |d| if d == "tag" { "tag" } else { "branch" },
            50,
        );
        let out = g.finish("git checkout fea", &[], &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(
            (out[0].insert.as_str(), out[0].source),
            ("feature-x", "branch")
        );
    }

    #[test]
    fn shell_directory_words_join_rifts_directory_rows() {
        let dir = scratch("dirdup");
        for d in ["src", "sample"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let line = "cd s";
        let mut g = gather(line, 4, dir.to_str(), None);
        // bash's `complete -d cd` lists directories without the `/`
        g.add(
            &pairs(&[("src", ""), ("sample", ""), ("sbin-only", "")]),
            "shell",
        );
        let out = g.finish(line, &[], &[]);
        let names: Vec<(&str, &str)> = out.iter().map(|c| (c.insert.as_str(), c.source)).collect();
        assert_eq!(
            names,
            vec![
                ("sbin-only", "shell"),
                ("sample/", "path"),
                ("src/", "path")
            ],
            "one row per directory, Rift's kept"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_remote_pane_lists_no_local_commands() {
        let dir = scratch("remotecmds");
        let exe = dir.join("lsfoo");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.as_os_str();
        // control: a local pane on that `$PATH` lists it
        let local = gather("lsf", 3, None, Some(path)).finish("lsf", &[], &[]);
        assert_eq!(inserts_of(&local, "command"), ["lsfoo"]);
        let remote = gather_remote("lsf", 3);
        assert!(remote.first_word());
        assert!(remote.finish("lsf", &[], &[]).is_empty());
        // nor this machine's own `$PATH` (control: it lists `ls…` commands)
        let here = gather("ls", 2, None, None).finish("ls", &[], &[]);
        assert!(!inserts_of(&here, "command").is_empty(), "{here:?}");
        let there = gather_remote("ls", 2).finish("ls", &[], &[]);
        assert!(inserts_of(&there, "command").is_empty(), "{there:?}");
        // the remote machine's own commands still merge in
        let mut g = gather_remote("lsf", 3);
        g.add_commands(&["lsfix".into()]);
        assert_eq!(inserts_of(&g.finish("lsf", &[], &[]), "command"), ["lsfix"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_names_match_as_typed_then_any_case_then_fuzzily() {
        let cmds: Vec<String> = ["Gimp", "convertformat", "gi", "git", "git-lfs", "tf-apply"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let names = |typed: &str| -> Vec<String> {
            command_completions(&cmds, typed)
                .into_iter()
                .map(|(c, _)| c.insert)
                .collect()
        };
        // one typed character is enough; the exact name is a row too
        assert_eq!(names("gi"), ["gi", "git", "git-lfs", "Gimp"]);
        assert_eq!(names("git"), ["git", "git-lfs"]);
        // a subsequence anywhere in the name (Warp's fuzzy match)
        assert_eq!(names("tfa"), ["convertformat", "tf-apply"]);
        // nothing typed: none listed (an empty word would list them all)
        assert!(names("").is_empty());
    }

    #[test]
    fn command_names_are_escaped() {
        let mut cmds: Vec<String> = [
            "zzprobe;echo X",
            "zzsafe",
            "zz q'\"$`\\",
            "zz\tctl",
            "zz sp",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        cmds.sort();
        let inserts = |token: &str| -> Vec<String> {
            command_completions(&cmds, token)
                .into_iter()
                .map(|(c, _)| c.insert)
                .collect()
        };
        let all = inserts("zz");
        assert!(all.contains(&"zzprobe\\;echo\\ X".to_string()));
        assert!(all.contains(&"zzsafe".to_string()));
        assert!(all.contains(&"zz\\ q\\'\\\"\\$\\`\\\\".to_string()));
        assert!(all.contains(&"zz\\ sp".to_string()));
        // control characters cannot be carried portably: not offered
        assert!(!all.iter().any(|s| s.contains('\t')));
        // a typed escaped prefix matches the real name
        assert_eq!(inserts("zz\\ s"), vec!["zz\\ sp".to_string()]);
        // round trip: the shell word unescapes to exactly the filename
        for name in &cmds {
            if let Some(w) = escape_word(name) {
                assert_eq!(&unescape_word(&w), name);
            }
        }
    }

    /// A PATH file named like an assignment completed bare set a shell
    /// variable instead of running the file.
    #[test]
    fn assignment_shaped_command_names_run_the_file() {
        let cmds: Vec<String> = ["zz-opt=v", "zzplain", "zzriftassign=value"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let all: Vec<String> = command_completions(&cmds, "zz")
            .into_iter()
            .map(|(c, _)| c.insert)
            .collect();
        assert!(all.contains(&"zzriftassign\\=value".to_string()), "{all:?}");
        // not a variable name: no assignment to fear, left as typed
        assert!(all.contains(&"zz-opt=v".to_string()), "{all:?}");
        assert!(all.contains(&"zzplain".to_string()));
        assert_eq!(escape_command("_A1=x y"), Some("_A1\\=x\\ y".into()));
        for name in &cmds {
            assert_eq!(&unescape_word(&escape_command(name).unwrap()), name);
        }

        // and the shells agree: the insert runs the file, sets no variable
        let dir = std::env::temp_dir().join(format!("rift-asg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("zzriftassign=value");
        std::fs::write(&file, "#!/bin/sh\necho RAN\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", dir.display());
        let line = "zzriftassign\\=value; echo \"var:$zzriftassign\"";
        for sh in ["bash", "zsh", "fish"] {
            let Ok(out) = std::process::Command::new(sh)
                .args(["-c", line])
                .env("PATH", &path)
                .output()
            else {
                continue; // shell not installed here
            };
            assert_eq!(String::from_utf8_lossy(&out.stdout), "RAN\nvar:\n", "{sh}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn history_prefix() {
        let history = vec!["git push origin main".to_string(), "ls -la".to_string()];
        let out = complete("git pu", 6, &history, &[], None);
        assert!(out.iter().any(|c| c.insert == "git push origin main"));
    }

    #[test]
    fn token_extraction() {
        assert_eq!(current_token("git che", 7), (4, "che"));
        assert_eq!(current_token("ls", 2), (0, "ls"));
        assert_eq!(current_token("cargo build ", 12), (12, ""));
    }

    #[test]
    fn token_respects_escapes_and_quotes() {
        assert_eq!(current_token("cat my\\ di", 10), (4, "my\\ di"));
        assert_eq!(current_token("cat 'my di", 10), (4, "'my di"));
        assert_eq!(current_token("cat \"a b\" c", 11), (10, "c"));
        assert_eq!(current_token("echo é f", 9), (8, "f"));
    }

    #[test]
    fn double_quotes_keep_backslashes_that_escape_nothing() {
        assert_eq!(unescape_word(r#""./a\b""#), r"./a\b");
        assert_eq!(unescape_word(r#""a\$b\`c\"d\\e""#), r#"a$b`c"d\e"#);
        assert_eq!(unescape_word(r"./a\b"), "./ab"); // unquoted: escapes any char
        assert_eq!(unescape_word(r"'./a\b'"), r"./a\b"); // single quotes: literal
        assert_eq!(unescape_word(r#""a\"#), r"a\"); // trailing backslash stays
    }

    #[test]
    fn quoted_backslash_prefix_picks_the_backslash_file() {
        let dir = std::env::temp_dir().join(format!("rift-complete-bs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(r"d\x")).unwrap();
        std::fs::write(dir.join(r"a\b.txt"), "").unwrap();
        std::fs::write(dir.join("ab.txt"), "").unwrap();
        std::fs::write(dir.join(r"d\x/in.txt"), "").unwrap();
        let cwd = dir.to_str();
        let paths = |buf: &str| -> Vec<String> {
            complete(buf, buf.len(), &[], &[], cwd)
                .into_iter()
                .filter(|c| c.source == "path")
                .map(|c| c.insert)
                .collect()
        };
        assert_eq!(paths(r#"cat "./a\b""#), [r"./a\\b.txt"]);
        assert_eq!(paths(r#"cat "./a\b"#), [r"./a\\b.txt"]);
        assert_eq!(paths(r"cat ./a\\b"), [r"./a\\b.txt"]);
        // the prefix first; `a\b.txt` holds `ab` as a subsequence
        assert_eq!(paths(r"cat ./ab"), ["./ab.txt", r"./a\\b.txt"]);
        assert_eq!(paths(r#"cat "./d\x/i"#), [r"./d\\x/in.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn token_end_spans_the_whole_word() {
        // closing quote right of the cursor belongs to the word
        let b = "cat \"./hel\"";
        assert_eq!(token_end(b, b.len() - 1), b.len());
        // mid-word: the suffix up to the next argument is part of the word
        let b = "cat ./hello.txt --other";
        assert_eq!(token_end(b, 9), 15);
        // end of buffer and at a space
        assert_eq!(token_end("cat ./hel", 9), 9);
        assert_eq!(token_end("cat  x", 4), 4);
        // quoted / escaped whitespace does not end the word
        let b = "cat 'my di' x";
        assert_eq!(token_end(b, 7), 11);
        let b = "cat my\\ di x";
        assert_eq!(token_end(b, 5), 10);
        // operators end it; whitespace before the cursor never counts
        assert_eq!(token_end("ls ./a|grep x", 5), 6);
        assert_eq!(token_end("echo é\u{3000}f", 7), 7);
    }

    #[test]
    fn escaped_words_round_trip() {
        for name in [
            "hello world.txt",
            "it's",
            "say \"hi\"",
            "$HOME;rm (x)",
            "back\\slash",
            "a&b|c<d>e*f?g[h]{i}!j#k`l^",
            "café",
            "=eq",
            "~tilde",
        ] {
            let escaped = escape_word(name).unwrap();
            assert!(
                !escaped.contains(' ') || escaped.contains("\\ "),
                "{escaped}"
            );
            assert_eq!(
                current_token(&escaped, escaped.len()),
                (0, escaped.as_str())
            );
            assert_eq!(unescape_word(&escaped), name, "{escaped}");
        }
        assert_eq!(escape_word("~/a b").unwrap(), "~/a\\ b");
        assert_eq!(escape_word("café/x-1.txt").unwrap(), "café/x-1.txt");
        assert_eq!(escape_word("line\nbreak"), None);
    }

    #[test]
    fn path_completion_inserts_one_shell_word() {
        let dir = std::env::temp_dir().join(format!("rift-complete-quote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("my dir")).unwrap();
        std::fs::write(dir.join("hello world.txt"), "").unwrap();
        std::fs::write(dir.join("it's;$x.txt"), "").unwrap();
        std::fs::write(dir.join("my dir/inner.txt"), "").unwrap();
        let cwd = dir.to_str();
        let path_inserts = |buf: &str| -> Vec<String> {
            complete(buf, buf.len(), &[], &[], cwd)
                .into_iter()
                .filter(|c| c.source == "path")
                .map(|c| c.insert)
                .collect()
        };
        assert_eq!(path_inserts("cat ./hel"), ["./hello\\ world.txt"]);
        assert_eq!(path_inserts("cat ./it"), ["./it\\'s\\;\\$x.txt"]);
        // directory continuation: the escaped prefix completes further
        assert_eq!(path_inserts("cat ./my"), ["./my\\ dir/"]);
        assert_eq!(path_inserts("cat ./my\\ dir/in"), ["./my\\ dir/inner.txt"]);
        // a quoted token resolves too, and is replaced by the escaped form
        assert_eq!(path_inserts("cat './my dir/in"), ["./my\\ dir/inner.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bare_tilde_does_not_panic() {
        // regression: "~" expanded to $HOME made file_part longer than the
        // token → slice-index underflow panic on every '~' keystroke
        complete("~", 1, &[], &[], None);
        complete("ls ~", 4, &[], &[], None);
        complete("ls ~ab", 6, &[], &[], None);
        complete("cat ~/", 6, &[], &[], None);
    }

    #[test]
    fn tilde_slash_expands_and_keeps_prefix() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        if std::fs::read_dir(&home).is_err() {
            return;
        }
        let out = complete("ls ~/", 5, &[], &[], Some("/"));
        // every path completion must re-use the user's "~/" prefix verbatim
        assert!(
            out.iter()
                .filter(|c| c.source == "path")
                .all(|c| c.insert.starts_with("~/"))
        );
    }

    #[test]
    fn path_commands_complete_first_word_only() {
        // `ls` exists on every unix PATH; one typed character completes
        let out = complete("l", 1, &[], &[], None);
        assert!(
            out.iter()
                .any(|c| c.source == "command" && c.insert.starts_with('l'))
        );
        let out = complete("ls", 2, &[], &[], None);
        assert!(out.iter().all(|c| c.source == "command"));
        // the exact name first (Warp's exact match), the longer names after
        assert_eq!(out[0].insert, "ls");
        // not the first word: no command completion
        let out = complete("echo ls", 7, &[], &[], None);
        assert!(out.iter().all(|c| c.source != "command"));
    }

    #[test]
    fn shell_history_feeds_ghost_after_pane_history() {
        let pane = vec!["git push origin main".to_string()];
        let shell = vec!["git pull".to_string(), "git push --force".to_string()];
        let out = complete("git pu", 6, &pane, &shell, None);
        let hist: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "history")
            .map(|c| c.insert.as_str())
            .collect();
        assert_eq!(
            hist,
            vec!["git push origin main", "git push --force", "git pull"]
        );
        // multi-line entries never become ghost text
        let shell = vec!["git pu\nll".to_string()];
        let out = complete("git pu", 6, &[], &shell, None);
        assert!(out.iter().all(|c| c.source != "history"));
    }

    #[test]
    fn ranked_history_comes_before_pane_and_shell_history() {
        use crate::command_history::{Entry, ranked_matches};
        let at = |cmd: &str, cwd: &str, finished| Entry {
            command: cmd.into(),
            cwd: Some(cwd.into()),
            repo: Some(cwd.into()),
            finished,
            exit: Some(0),
            session: None,
            seq: None,
        };
        // the newer command ran in another directory: the one run here wins
        let store = vec![at("pnpm dev", "/a", 1), at("pnpm dev:web", "/b", 2)];
        let ranked = ranked_matches(&store, "pnpm de", Some("/a"), Some("/a"));
        let pane = vec!["pnpm dev:web".to_string(), "pnpm deploy".to_string()];
        let shell = vec!["pnpm dev".to_string(), "pnpm dedupe".to_string()];
        let out = gather("pnpm de", 7, None, None).finish_ranked("pnpm de", &ranked, &pane, &shell);
        assert_eq!(
            inserts_of(&out, "history"),
            ["pnpm dev", "pnpm dev:web", "pnpm deploy", "pnpm dedupe"]
        );
    }

    #[test]
    fn history_lines_naming_a_missing_file_are_skipped() {
        let dir = scratch("valid");
        std::fs::write(dir.join("here.txt"), "").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        let cwd = dir.to_str().unwrap();
        let ranked = ["cat gone.txt", "cat here.txt"];
        let out =
            gather("cat ", 4, None, None).finish_checked("cat ", &ranked, &[], &[], Some(cwd));
        assert_eq!(inserts_of(&out, "history"), ["cat here.txt"]);
        // the ghost is the first history line: the next match
        assert_eq!(
            out.iter().find(|c| c.source == "history").unwrap().insert,
            "cat here.txt"
        );
        // pane and shell history are checked too
        let pane = vec!["cat here.txt".to_string(), "cat gone.txt".to_string()];
        let out = gather("cat ", 4, None, None).finish_checked("cat ", &[], &pane, &[], Some(cwd));
        assert_eq!(inserts_of(&out, "history"), ["cat here.txt"]);
        // a remote pane (no local directory) checks nothing
        let out = gather("cat ", 4, None, None).finish_checked("cat ", &ranked, &[], &[], None);
        assert_eq!(inserts_of(&out, "history"), ranked);
        for (line, valid) in [
            ("cat gone.txt", false),
            ("cat here.txt", true),
            ("cat sub/../here.txt", true),
            ("cd sub", true),
            ("cd here.txt", false),
            ("cd gone", false),
            ("cd", true),
            ("cd -", true),
            ("ls -la gone", true), // an option the spec does not know
            ("ls --all gone", false),
            ("ls here.txt gone", true), // only the first positional
            ("git add gone.txt", true), // stages a deletion
            ("cargo new gone", true),
            ("grep x here.txt", true),
            ("echo hi && cat gone.txt", false),
            ("cat gone.txt; ls", false),
            ("cat here.txt | grep x gone", true),
            ("cat $HOME/x", true),
            ("cat *.md", true),
            ("cat ~nobody/x", true),
            ("scp host:gone.txt .", true),
            ("cat > gone.txt", true),
            ("python3 -m http.server", true),
            ("frobnicate gone.txt", true), // no Rift spec: kept
            ("bash scripts/db.sh backup", false),
        ] {
            assert_eq!(command_valid(line, cwd), valid, "{line}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_script_runners_script_must_be_a_file_here() {
        let dir = scratch("scripts");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/prisma-admin.sh"), "").unwrap();
        std::fs::write(dir.join("app.js"), "").unwrap();
        let cwd = dir.to_str().unwrap();
        for (line, valid) in [
            ("bash scripts/prisma-admin.sh reset --force", true),
            ("bash scripts/gone.sh reset", false),
            ("bash scripts", false), // a directory is no script
            ("sh -x scripts/prisma-admin.sh", true),
            ("sh -x scripts/gone.sh", false),
            ("zsh -o errexit scripts/gone.sh", false),
            ("zsh -o errexit scripts/prisma-admin.sh", true),
            ("bash -c 'rm gone.sh'", true), // inline code
            ("bash -xc 'echo'", true),
            ("bash -s arg", true), // the script comes on stdin
            ("bash < gone.sh", true),
            ("bash", true),
            ("bash -- gone.sh", false),
            ("fish -c 'ls gone'", true),
            ("fish gone.fish", false),
            ("node app.js", true),
            ("node -r dotenv/config app.js", true),
            ("node -r dotenv/config gone.js", false),
            ("node -e 'require(\"gone\")'", true),
            ("node --version", true),
            ("ruby gone.rb", false),
            ("ruby -e 'p 1'", true),
            ("perl -I lib gone.pl", false),
            ("source gone.env", false),
            (". ./scripts/prisma-admin.sh", true),
            (". gone/activate", false),
            ("bash $SCRIPT", true),
            ("bash ~nobody/x.sh", true),
            ("echo hi && bash scripts/gone.sh", false),
        ] {
            assert_eq!(command_valid(line, cwd), valid, "{line}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_checks_stop_at_the_budget() {
        // past the budget every line is kept unchecked
        let dir = scratch("budget");
        let cwd = dir.to_str().unwrap();
        let lines: Vec<String> = (0..5).map(|i| format!("cat gone{i}.txt")).collect();
        let ranked: Vec<&str> = lines.iter().map(String::as_str).collect();
        let finish = |deadline| {
            let out = gather("cat ", 4, None, None).finish_checked_until(
                "cat ",
                &ranked,
                &[],
                &[],
                Some(cwd),
                deadline,
                &|_| false,
            );
            inserts_of(&out, "history").len()
        };
        assert_eq!(finish(Instant::now() + VALIDATE_BUDGET), 0);
        assert_eq!(finish(Instant::now()), 5);
        assert_eq!(VALIDATE_BUDGET, Duration::from_millis(150));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignored_lines_leave_every_history_source_and_free_their_place() {
        // Rift's ranked history, the pane's own, the imported shell's: an
        // ignored line is no history row from any of them (exact lines:
        // another line starting with it stays)
        let ignored = ["git push --force", "git pull", "git status"];
        let is_ignored = |l: &str| ignored.contains(&l);
        let ranked = ["git push --force", "git push"];
        let pane = vec!["git pull".to_string(), "git pull --rebase".to_string()];
        // (newest last, as imported)
        let shell = vec!["git stash".to_string(), "git status".to_string()];
        let out = gather("git ", 4, None, None).finish_checked_except(
            "git ",
            &ranked,
            &pane,
            &shell,
            None,
            &is_ignored,
        );
        let rows = inserts_of(&out, "history");
        assert_eq!(
            rows,
            ["git push", "git pull --rebase", "git stash"],
            "{rows:?}"
        );
        // unfiltered, they all show
        let all =
            gather("git ", 4, None, None).finish_checked("git ", &ranked, &pane, &shell, None);
        assert!(
            ignored
                .iter()
                .all(|l| inserts_of(&all, "history").contains(&l.to_string())),
            "{all:?}"
        );
        // ignored lines hold no place: older lines fill the rows
        let many: Vec<String> = (0..MAX_HISTORY * 2).map(|i| format!("echo {i}")).collect();
        let first: Vec<&str> = many
            .iter()
            .rev()
            .take(MAX_HISTORY)
            .map(String::as_str)
            .collect();
        let out = gather("echo ", 5, None, None).finish_checked_except(
            "echo ",
            &[],
            &many,
            &[],
            None,
            &|l| first.contains(&l),
        );
        let rows = inserts_of(&out, "history");
        assert_eq!(rows.len(), MAX_HISTORY);
        assert!(
            rows.iter().all(|r| !first.contains(&r.as_str())),
            "{rows:?}"
        );
    }

    #[test]
    fn empty_token_after_space_lists_subcommands() {
        let out = complete("git ", 4, &[], &[], None);
        let subs: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "shell")
            .map(|c| c.insert.as_str())
            .collect();
        assert!(subs.contains(&"commit"));
        assert!(subs.contains(&"status"));
    }

    #[test]
    fn nested_subcommand_completion() {
        let out = complete("docker compose ", 15, &[], &[], None);
        let subs: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "shell")
            .map(|c| c.insert.as_str())
            .collect();
        assert!(subs.contains(&"up"), "docker compose <sub>: {subs:?}");
        assert!(subs.contains(&"down"));
    }

    #[test]
    fn option_completion() {
        // "-" lists every option (long form preferred)
        let out = complete("git commit -", 12, &[], &[], None);
        assert!(
            out.iter().any(|c| c.source == "option"
                && c.insert == "--message"
                && !c.description.is_empty())
        );
        // "--a" matches long forms by prefix first, then as a subsequence
        let out = complete("git commit --a", 14, &[], &[], None);
        assert!(
            out.iter()
                .any(|c| c.source == "option" && c.insert == "--amend")
        );
        let opts: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "option")
            .map(|c| c.insert.as_str())
            .collect();
        let prefixed = opts.iter().take_while(|o| o.starts_with("--a")).count();
        assert!(
            prefixed > 0 && opts[prefixed..].iter().all(|o| !o.starts_with("--a")),
            "{opts:?}"
        );
        // a non-repeatable flag already present is not offered again
        let out = complete("git commit --amend --a", 22, &[], &[], None);
        assert!(out.iter().all(|c| c.insert != "--amend"));
    }

    #[test]
    fn pipe_starts_a_new_command() {
        // subcommand completion resolves against the command after the pipe
        let out = complete("cat foo | git ch", 16, &[], &[], None);
        let names: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "shell")
            .map(|c| c.insert.as_str())
            .collect();
        assert!(names.contains(&"checkout"), "after pipe: {names:?}");
        assert!(!names.contains(&"commit"));
        // and `;` / `&&` behave the same
        let out = complete("x=1; git ch", 11, &[], &[], None);
        assert!(
            out.iter()
                .any(|c| c.source == "shell" && c.insert == "checkout")
        );
    }

    #[test]
    fn git_checkout_completes_refs_from_repo() {
        // a temp dir with fake git ref files; no real git invoked
        let dir = std::env::temp_dir().join(format!(
            "rift-complete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(dir.join(".git/refs/heads")).unwrap();
        std::fs::write(dir.join(".git/refs/heads/main"), "0\n").unwrap();
        std::fs::write(dir.join(".git/refs/heads/develop"), "0\n").unwrap();

        let cwd = dir.to_str().unwrap();
        let out = complete("git checkout ", 13, &[], &[], Some(cwd));
        let branches: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "branch")
            .map(|c| c.insert.as_str())
            .collect();
        assert!(branches.contains(&"main"), "refs: {branches:?}");
        assert!(branches.contains(&"develop"));
        // prefix filtering still applies
        let out = complete("git checkout de", 15, &[], &[], Some(cwd));
        assert!(out.iter().any(|c| c.insert == "develop"));
        assert!(
            out.iter()
                .all(|c| c.source != "branch" || c.insert.starts_with("de"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rift-complete-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn generator_values_are_escaped_and_matched_unescaped() {
        let dir = scratch("inject");
        let heads = dir.join(".git/refs/heads");
        std::fs::create_dir_all(&heads).unwrap();
        let evil = "x$(touch${IFS}pwn);id";
        std::fs::write(heads.join(evil), "0\n").unwrap();
        let cwd = dir.to_str();
        let want = escape_word(evil).unwrap();
        assert!(!want.contains("$(") || want.contains("\\$\\("));
        for buf in [
            "git checkout x",
            "git checkout x\\$\\(to",
            "git checkout 'x$(to",
        ] {
            let out = complete(buf, buf.len(), &[], &[], cwd);
            let got: Vec<&str> = out
                .iter()
                .filter(|c| c.source == "branch")
                .map(|c| c.insert.as_str())
                .collect();
            assert_eq!(got, [want.as_str()], "{buf}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_and_remote_generators_escape_too() {
        let dir = scratch("inject2");
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts": {"x;touch pwn": "echo", "ok": "echo"}}"#,
        )
        .unwrap();
        let cwd = dir.to_str();
        let out = complete("npm run x", 9, &[], &[], cwd);
        let got: Vec<&str> = out
            .iter()
            .filter(|c| c.source == "script")
            .map(|c| c.insert.as_str())
            .collect();
        assert_eq!(got, ["x\\;touch\\ pwn"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dir_completion_has_no_files_and_no_duplicates() {
        let dir = scratch("dirs");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("file.txt"), "").unwrap();
        let cwd = dir.to_str();
        let inserts = |buf: &str| -> Vec<String> {
            complete(buf, buf.len(), &[], &[], cwd)
                .into_iter()
                .map(|c| c.insert)
                .collect()
        };
        assert_eq!(inserts("cd ./"), ["./sub/"]);
        assert_eq!(inserts("cat ./"), ["./file.txt", "./sub/"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_complete_as_directories() {
        let dir = scratch("symlink");
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let cwd = dir.to_str();
        let out = complete("cd ./li", 7, &[], &[], cwd);
        assert_eq!(
            out.iter().map(|c| c.insert.as_str()).collect::<Vec<_>>(),
            ["./link/"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn option_with_a_value_is_not_completed_as_an_option() {
        let out = complete("git commit --mess", 17, &[], &[], None);
        assert!(out.iter().any(|c| c.insert == "--message"));
        let out = complete("git commit --message=al", 23, &[], &[], None);
        assert!(out.iter().all(|c| c.source != "option"), "{out:?}");
    }

    use crate::completion_cache::tests::age;

    fn inserts_of(out: &[Completion], source: &str) -> Vec<String> {
        out.iter()
            .filter(|c| c.source == source)
            .map(|c| c.insert.clone())
            .collect()
    }

    #[test]
    fn match_kinds() {
        assert_eq!(match_kind("checkout", "che"), Some(Match::Prefix));
        assert_eq!(match_kind("checkout", ""), Some(Match::Prefix));
        assert_eq!(match_kind("checkout", "checkout"), Some(Match::Prefix));
        // any case is a prefix
        assert_eq!(match_kind("Makefile", "mak"), Some(Match::Prefix));
        assert_eq!(match_kind("cherry-pick", "pick"), Some(Match::Subsequence));
        assert_eq!(match_kind("b_u_i.md", "bui"), Some(Match::Subsequence));
        // one character is enough for a subsequence
        assert_eq!(match_kind("cherry-pick", "k"), Some(Match::Subsequence));
        assert_eq!(match_kind("checkout", "xyz"), None);
        // punctuation alone matches by prefix only
        assert_eq!(match_kind("a-b", "-"), None);
        assert_eq!(match_kind("-b", "-"), Some(Match::Prefix));
    }

    fn row(insert: &str) -> Completion {
        Completion {
            insert: insert.into(),
            description: String::new(),
            source: "shell",
        }
    }

    fn rows(inserts: &[&str]) -> Vec<Completion> {
        inserts.iter().map(|s| row(s)).collect()
    }

    #[test]
    fn menu_rows_are_exact_then_prefix_then_fuzzy_by_score() {
        let items = rows(&["stash", "Status", "st", "ST", "staged", "list-tags", "sort"]);
        let order: Vec<&str> = filter_by_query(&items, "st")
            .into_iter()
            .map(|(i, _, _)| items[i].insert.as_str())
            .collect();
        // exact as typed, exact in another case, prefixes in their order,
        // then subsequences by score (a run at a word start beats a gap)
        assert_eq!(
            order,
            ["st", "ST", "stash", "Status", "staged", "sort", "list-tags"]
        );
        // the matched characters: the prefix, or where the subsequence sat
        let got = filter_by_query(&items, "st");
        assert_eq!(got[2].2, vec![0, 1]);
        let lt = got
            .iter()
            .find(|(i, _, _)| items[*i].insert == "list-tags")
            .unwrap();
        assert!(matches!(lt.1, MenuMatch::Fuzzy { .. }));
        assert_eq!(lt.2, vec![2, 3]);
        // no letter or digit: prefixes only
        let dashes = rows(&["--all", "-a", "x-y"]);
        let got: Vec<usize> = filter_by_query(&dashes, "-")
            .into_iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(got, [0, 1]);
    }

    #[test]
    fn a_quoted_or_escaped_word_matches_the_escaped_insert() {
        // `cat "./my` against `./my\ file.txt`: a prefix of the literal text
        let (kind, at) = menu_match(r"./my\ file.txt", "\"./my\"").unwrap();
        assert_eq!(kind, MenuMatch::Prefix { cs: true });
        assert_eq!(at, vec![0, 1, 2, 3]);
        let (kind, at) = menu_match(r"./my\ file.txt", r"./my\ f").unwrap();
        assert_eq!(kind, MenuMatch::Prefix { cs: true });
        assert_eq!(at, (0..7).collect::<Vec<_>>());
        // the space typed quoted maps onto the escaped one (index 5)
        let (_, at) = menu_match(r"./my\ file.txt", "'./my f'").unwrap();
        assert_eq!(at, vec![0, 1, 2, 3, 5, 6]);
        assert_eq!(
            tab_action(&rows(&[r"./my\ file.txt"]), "\"./my\""),
            TabAction::Insert(0)
        );
        assert!(menu_match("abc", "\"x\"").is_none());
    }

    #[test]
    fn fuzzy_scores_prefer_word_starts_and_runs() {
        let (word_start, at) = fuzzy_score("git-checkout", "gco").unwrap();
        assert_eq!(at, vec![0, 4, 9]);
        let (middle, _) = fuzzy_score("ago-cto", "gco").unwrap();
        assert!(word_start > middle, "{word_start} {middle}");
        let (run, _) = fuzzy_score("xabcx", "abc").unwrap();
        let (spread, _) = fuzzy_score("xaxbxc", "abc").unwrap();
        assert!(run > spread, "{run} {spread}");
        assert_eq!(fuzzy_score("abc", "acb"), None);
        assert_eq!(fuzzy_score("ÄBC", "äc").map(|r| r.1), Some(vec![0, 2]));
    }

    #[test]
    fn tab_inserts_one_match_a_common_prefix_or_opens_the_menu() {
        // none
        assert_eq!(tab_action(&rows(&["main"]), "x"), TabAction::Nothing);
        // one as typed (a fuzzy row besides does not count)
        assert_eq!(
            tab_action(&rows(&["checkout", "xchx"]), "ch"),
            TabAction::Insert(0)
        );
        // none as typed, one in another case
        assert_eq!(
            tab_action(&rows(&["Makefile", "zmak"]), "mak"),
            TabAction::Insert(0)
        );
        // two as typed beat one in another case
        assert_eq!(
            tab_action(&rows(&["checkout", "cherry-pick", "Check"]), "ch"),
            TabAction::CommonPrefix("che".into())
        );
        // the common prefix is no longer than the word: the menu
        assert_eq!(
            tab_action(&rows(&["stash", "status"]), "sta"),
            TabAction::Open
        );
        // an exact match counts: `git` + `gitk` opens the menu
        assert_eq!(tab_action(&rows(&["git", "gitk"]), "git"), TabAction::Open);
        // only fuzzy rows: the menu
        assert_eq!(
            tab_action(&rows(&["cherry-pick", "pick"]), "pk"),
            TabAction::Open
        );
    }

    #[test]
    fn tab_in_the_menu_completes_the_common_prefix_or_moves_on() {
        let p = |cs| MenuMatch::Prefix { cs };
        let fz = MenuMatch::Fuzzy { score: 1 };
        assert_eq!(
            menu_tab(
                &[("checkout", p(true)), ("cherry", p(true)), ("xch", fz)],
                "ch"
            ),
            Some(("che".into(), false))
        );
        // one row as typed left: it is inserted and the menu closes
        assert_eq!(
            menu_tab(&[("checkout", p(true)), ("Chx", p(false))], "ch"),
            Some(("checkout".into(), true))
        );
        // nothing longer than the word: the next row
        assert_eq!(
            menu_tab(&[("stash", p(true)), ("status", p(true))], "sta"),
            None
        );
        assert_eq!(menu_tab(&[("Stash", p(false))], "st"), None);
    }

    #[test]
    fn a_space_follows_an_accepted_completion_at_the_end_only() {
        assert!(space_after("checkout", true, false));
        assert!(!space_after("src/", true, false));
        assert!(!space_after("--color=", true, false));
        assert!(!space_after("checkout", false, false));
        assert!(!space_after("checkout", true, true));
    }

    #[test]
    fn enter_accepts_unless_as_you_type_already_has_it() {
        assert!(enter_accepts(false, "che", Some("checkout")));
        assert!(enter_accepts(false, "checkout", Some("checkout")));
        assert!(!enter_accepts(false, "che", None));
        assert!(enter_accepts(true, "che", Some("checkout")));
        assert!(!enter_accepts(true, "checkout", Some("checkout")));
        assert!(!enter_accepts(true, "src", Some("src/")));
        assert!(enter_accepts(true, "sr", Some("src/")));
    }

    #[test]
    fn common_prefix_stays_on_character_boundaries() {
        assert_eq!(common_prefix(["añb", "añc"].into_iter()), Some("añ"));
        assert_eq!(common_prefix(["aé", "aè"].into_iter()), Some("a"));
        assert_eq!(common_prefix(["x"].into_iter()), Some("x"));
        assert_eq!(common_prefix(std::iter::empty()), None);
    }

    #[test]
    fn ranking_is_prefix_then_subsequence() {
        let dir = scratch("rank");
        for f in [
            "b_u_i_l_d.md",
            "my-build.txt",
            "Build.md",
            "build.rs",
            "other",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let out = complete("cat ./bui", 9, &[], &[], dir.to_str());
        // prefixes in any case (name order), then subsequences by score: a
        // run from a word start first
        assert_eq!(
            inserts_of(&out, "path"),
            [
                "./Build.md",
                "./build.rs",
                "./my-build.txt",
                "./b_u_i_l_d.md"
            ]
        );
        // the same request gives the same order
        assert_eq!(out, complete("cat ./bui", 9, &[], &[], dir.to_str()));
        // subcommands too: the best subsequence first
        let out = complete("git pick", 8, &[], &[], None);
        assert_eq!(
            inserts_of(&out, "shell").first().map(String::as_str),
            Some("cherry-pick")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recency_orders_equal_matches_and_source_comes_first() {
        let dir = scratch("recent");
        for f in ["alpha.txt", "beta.txt", "gamma.txt"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let cwd = dir.to_str();
        let history = vec!["cat ./gamma.txt".to_string(), "cat ./beta.txt".to_string()];
        let out = complete("cat ./", 6, &history, &[], cwd);
        // newest use first, then older, then unused in name order
        assert_eq!(
            inserts_of(&out, "path"),
            ["./beta.txt", "./gamma.txt", "./alpha.txt"]
        );
        // at an equal match, source beats recency; match kind beats source
        let word = |insert: &str, source: &'static str, m: Match| {
            (
                Completion {
                    insert: insert.into(),
                    description: String::new(),
                    source,
                },
                m,
            )
        };
        let mut words = vec![
            word("./x", "path", Match::Prefix),
            word("xs", "shell", Match::Subsequence),
            word("xb", "branch", Match::Prefix),
            word("xa", "branch", Match::Prefix),
        ];
        rank(&mut words, "x", &["git checkout ./x".to_string()], &[]);
        let order: Vec<&str> = words.iter().map(|(c, _)| c.insert.as_str()).collect();
        // xb before xa: generator order kept (stable) when neither was used
        assert_eq!(order, ["xb", "xa", "./x", "xs"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_lines_survive_a_full_word_list_and_unique_prefix_stays_first() {
        let dir = scratch("uniq");
        let heads = dir.join(".git/refs/heads");
        std::fs::create_dir_all(heads.join("feature")).unwrap();
        std::fs::write(heads.join("main"), "0\n").unwrap();
        std::fs::write(heads.join("feature/main-fix"), "0\n").unwrap();
        for i in 0..80 {
            std::fs::write(heads.join(format!("zz{i:02}")), "0\n").unwrap();
        }
        let cwd = dir.to_str();
        // Tab takes the first candidate: the prefix match, as before
        let out = complete("git checkout mai", 16, &[], &[], cwd);
        assert_eq!(out[0].insert, "main");
        assert!(out.iter().any(|c| c.insert == "feature/main-fix"));
        // 80 refs fill the word list; the history line still comes back
        let history = vec!["git checkout zz07 && make".to_string()];
        let out = complete("git checkout zz", 15, &history, &[], cwd);
        assert!(out.len() <= 50);
        assert_eq!(inserts_of(&out, "history"), ["git checkout zz07 && make"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_listing_is_cached_until_the_directory_changes() {
        let dir = scratch("pathcache");
        std::fs::write(dir.join("one.txt"), "").unwrap();
        age(&dir, 100);
        let cwd = dir.to_str();
        assert_eq!(
            inserts_of(&complete("cat ./", 6, &[], &[], cwd), "path"),
            ["./one.txt"]
        );
        std::fs::write(dir.join("two.txt"), "").unwrap();
        age(&dir, 90);
        assert_eq!(
            inserts_of(&complete("cat ./", 6, &[], &[], cwd), "path"),
            ["./one.txt", "./two.txt"]
        );
        // a directory over the path cap: `cd` still lists its directories
        for i in 0..40 {
            std::fs::write(dir.join(format!("a{i:02}.txt")), "").unwrap();
        }
        std::fs::create_dir(dir.join("zsub")).unwrap();
        assert_eq!(
            inserts_of(&complete("cd ./", 5, &[], &[], cwd), "path"),
            ["./zsub/"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_commands_rescan_on_a_new_path_or_a_changed_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("pathcmds");
        let other = scratch("pathcmds2");
        let exe = |d: &std::path::Path, name: &str| {
            let f = d.join(name);
            std::fs::write(&f, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        exe(&dir, "zzfirst");
        exe(&other, "zzother");
        age(&dir, 100);
        let path = dir.clone().into_os_string();
        let t0 = Instant::now();
        assert_eq!(*commands_on_at(&path, t0), ["zzfirst"]);
        exe(&dir, "zzsecond");
        age(&dir, 90);
        // within the re-check interval: the cached scan, no stat at all
        assert_eq!(
            *commands_on_at(&path, t0 + Duration::from_secs(1)),
            ["zzfirst"]
        );
        // after it: the directory's stamp changed, so it is scanned again
        assert_eq!(
            *commands_on_at(&path, t0 + Duration::from_secs(3)),
            ["zzfirst", "zzsecond"]
        );
        // a different $PATH value (rift-use, a new shell probe): at once
        let both = std::env::join_paths([&other, &dir]).unwrap();
        assert_eq!(
            *commands_on_at(&both, t0 + Duration::from_secs(3)),
            ["zzfirst", "zzother", "zzsecond"]
        );
        // and completion takes the pane's $PATH when given
        let out = complete_on_path("zzot", 4, &[], &[], None, Some(&both));
        assert_eq!(inserts_of(&out, "command"), ["zzother"]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn no_local_directory_reads_no_local_files() {
        let dir = scratch("nolocal");
        std::fs::create_dir_all(dir.join(".git/refs/heads")).unwrap();
        std::fs::write(dir.join(".git/refs/heads/main"), "0\n").unwrap();
        std::fs::write(dir.join("Makefile"), "build:\n").unwrap();
        let abs = format!("cat {}/", dir.display());
        for buf in [abs.as_str(), "cat ./", "cat ~/", "make ", "cd /"] {
            let out = complete(buf, buf.len(), &[], &[], None);
            assert!(
                out.iter()
                    .all(|c| c.source != "path" && c.source != "make-target"),
                "{buf}: {out:?}"
            );
        }
        // with the directory, the same requests complete
        assert!(!complete(&abs, abs.len(), &[], &[], dir.to_str()).is_empty());
        let out = complete("make ", 5, &[], &[], dir.to_str());
        assert_eq!(inserts_of(&out, "make-target"), ["build"]);
        // history still completes
        let out = complete("cat ./", 6, &["cat ./x".to_string()], &[], None);
        assert_eq!(inserts_of(&out, "history"), ["cat ./x"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn redirects_and_newlines_bound_the_token_and_the_command() {
        assert_eq!(current_token("cat a >fi", 9), (7, "fi"));
        assert_eq!(current_token("a|ls", 4), (2, "ls"));
        assert_eq!(current_token("cat 'a>b", 8), (4, "'a>b"));
        let out = complete("ls\ngit ch", 9, &[], &[], None);
        assert!(
            out.iter()
                .any(|c| c.source == "shell" && c.insert == "checkout")
        );
    }
}
