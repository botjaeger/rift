//! Warp-style vim modal editing for the owned command-line editor.
//!
//! The buffer is usually one line but may hold several (Shift+Enter). `0`
//! `$` `^` `_` `I` `A` `%` `J` `o` `O` `gg` `G` `>` `<` and visual-line mode
//! work on the lines around the cursor; the other motions see the buffer as
//! one run of characters (a newline is a blank to them), and `dd` / `cc` /
//! `yy` act on the whole buffer. `j` / `k` recall history in Normal mode and
//! move between lines only in the visual modes. In Normal mode the cursor sits
//! *on* a character — it is the byte offset of that char and is clamped to the
//! last char whenever the buffer is non-empty (0 when it is empty).
//!
//! The visual modes reuse the editor's selection (`anchor`): the selection is
//! the inclusive char range between the anchor and the cursor (whole lines in
//! visual-line mode), so drawing and ⌘C see exactly what vim operates on.
//!
//! Every mutation goes through [`EditorState::replace_range`], the single
//! buffer-mutation point that pushes undo snapshots. A change made of several
//! mutations (an Insert or Replace session, a visual operator, a `.` repeat)
//! is folded into one undo step, so one `u` undoes one change; ⌃R redoes
//! and `U` puts back the last changed line.
//!
//! Registers: the unnamed one, `"a`–`"z` (`"A`–`"Z` append), `"0` (the last
//! yank), `"_` (black hole) and `"+` / `"*` (the system clipboard, which the
//! app hands in and takes out around each key: [`EditorState::vim_set_clipboard`],
//! [`EditorState::vim_take_clipboard`]).

use std::collections::HashMap;
use std::ops::Range;

use crate::state::EditorState;

/// Which vim mode the owned editor is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VimMode {
    #[default]
    Insert,
    Normal,
    /// `v`: charwise selection.
    Visual,
    /// `V`: linewise selection.
    VisualLine,
    /// `R`: typed chars overwrite the buffer.
    Replace,
}

/// A key handed to [`EditorState::vim_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimKey {
    Char(char),
    Escape,
    Enter,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    /// ⌃R: redo.
    Redo,
}

/// What the app should do with a key after vim looked at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimOutcome {
    /// Vim handled it; nothing else to do.
    Consumed,
    /// Vim did not handle it — run the normal editor path.
    PassThrough,
    /// Run the command (`submit()`).
    Submit,
    /// Recall the previous history entry (`history_up()`).
    HistoryUp,
    /// Recall the next history entry (`history_down()`).
    HistoryDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Delete,
    Change,
    Yank,
    /// `gu`
    Lower,
    /// `gU`
    Upper,
    /// `g~`
    ToggleCase,
    /// `>`: two spaces in front of every non-empty line.
    Indent,
    /// `<`: one leading tab, else up to two leading spaces, off every line.
    Outdent,
}

impl Op {
    /// The key that doubles a `g` case operator into its linewise form
    /// (`guu`, `gUU`, `g~~`).
    fn case_key(self) -> Option<char> {
        match self {
            Op::Lower => Some('u'),
            Op::Upper => Some('U'),
            Op::ToggleCase => Some('~'),
            _ => None,
        }
    }

    /// Ops that always act on whole lines.
    fn linewise(self) -> bool {
        matches!(self, Op::Indent | Op::Outdent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindKind {
    /// `f` — forward, land on the char.
    ForwardTo,
    /// `t` — forward, land just before the char.
    ForwardTill,
    /// `F` — backward, land on the char.
    BackTo,
    /// `T` — backward, land just after the char.
    BackTill,
}

impl FindKind {
    /// The opposite direction, for `,`.
    fn reversed(self) -> FindKind {
        match self {
            FindKind::ForwardTo => FindKind::BackTo,
            FindKind::ForwardTill => FindKind::BackTill,
            FindKind::BackTo => FindKind::ForwardTo,
            FindKind::BackTill => FindKind::ForwardTill,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Motion {
    Left,
    Right,
    /// `0`: start of the current line.
    LineStart,
    /// `$`: last char of the current line.
    LineEnd,
    /// `^`: first non-blank of the current line.
    FirstNonBlank,
    /// `w` / `W` (`true` = whitespace-delimited).
    WordFwd(bool),
    /// `b` / `B`.
    WordBack(bool),
    /// `e` / `E`.
    WordEnd(bool),
    /// `ge` / `gE`.
    WordEndBack(bool),
    Find(FindKind, char),
    /// `gg` / `G`: 1-based line number, `None` = the last line. Linewise.
    GotoLine(Option<usize>),
    /// `_`: first non-blank, count-1 lines down. Linewise.
    LineFirstNonBlank,
    /// `j` / `k` in the visual modes (same column). Linewise.
    LineDown,
    LineUp,
    /// An already-resolved target `(char index, inclusive)`: `%`, search.
    Jump(usize, bool),
}

impl Motion {
    fn linewise(self) -> bool {
        matches!(
            self,
            Motion::GotoLine(_) | Motion::LineFirstNonBlank | Motion::LineDown | Motion::LineUp
        )
    }
}

/// A key that still needs a char argument (`r`, `f`, `F`, `t`, `T`, `g`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Awaiting {
    Replace,
    Find(FindKind),
    /// `i` / `a` after an operator or in visual mode: the text object's char
    /// (`true` = inner).
    Object(bool),
    /// `g`: `gg` `ge` `gE` `gu` `gU` `g~`.
    G,
    /// `"`: the register's name.
    Register,
}

/// Operator / count state accumulated across keystrokes.
#[derive(Debug, Default, Clone)]
struct Pending {
    /// Count typed before the operator (the `2` in `2d3w`).
    count: Option<usize>,
    /// Count typed after the operator (the `3` in `2d3w`).
    op_count: Option<usize>,
    op: Option<Op>,
    awaiting: Option<Awaiting>,
}

const COUNT_CAP: usize = 10_000;

impl Pending {
    fn reset(&mut self) {
        *self = Pending::default();
    }

    fn idle(&self) -> bool {
        self.count.is_none()
            && self.op_count.is_none()
            && self.op.is_none()
            && self.awaiting.is_none()
    }

    /// Effective count: vim multiplies the two (`2d3w` = six words).
    fn total(&self) -> usize {
        self.count
            .unwrap_or(1)
            .saturating_mul(self.op_count.unwrap_or(1))
            .max(1)
    }

    /// The count, only when one was typed (`G` vs `3G`).
    fn given(&self) -> Option<usize> {
        (self.count.is_some() || self.op_count.is_some()).then(|| self.total())
    }
}

/// The last buffer change, replayed by `.`: the keys that made it and the
/// text typed in the Insert session it opened (empty when none).
#[derive(Debug, Clone)]
struct Change {
    keys: Vec<VimKey>,
    insert: String,
}

/// Modal-editing state hung off [`EditorState`]; `None` there = vim disabled.
#[derive(Debug, Default)]
pub struct VimState {
    pub mode: VimMode,
    pending: Pending,
    /// Unnamed register (`x`, `d`, `c`, `y` fill it; `p`/`P` read it).
    register: String,
    /// The register holds whole lines (`dd`, `cc`, `yy`, `V` operators).
    register_linewise: bool,
    /// Last `f`/`F`/`t`/`T` target, repeated by `;` and reversed by `,`.
    last_find: Option<(FindKind, char)>,
    /// Last `/` / `?` pattern and its direction (`true` = forward).
    last_search: Option<(String, bool)>,
    /// The open `/` / `?` prompt: direction and the text typed so far.
    prompt: Option<(bool, String)>,
    /// Keys of the command being typed; kept for `.` when it changes the buffer.
    rec: Vec<VimKey>,
    /// The buffer when the command began.
    before: String,
    /// Undo-stack depth and cursor when the command began (see `squash_undo`).
    mark: usize,
    mark_cursor: usize,
    /// Buffer and cursor when vim opened the current Insert session, to
    /// recover the typed text when it ends.
    insert_from: Option<(String, usize)>,
    last_change: Option<Change>,
    /// `.` is feeding keys: they are not recorded.
    replaying: bool,
    /// This command never becomes the last change (`u`, `U`, `.`).
    no_record: bool,
    /// `R`: the char each typed char overwrote (`None` = appended), so
    /// Backspace can put it back.
    replaced: Vec<Option<char>>,
    /// Named registers `"a`–`"z` and the yank register `"0`: text, linewise.
    regs: HashMap<char, (String, bool)>,
    /// The register the command being typed named (`"a`), until it is used.
    reg_sel: Option<char>,
    /// `reg_sel` was named by this very key (it outlives the key).
    reg_fresh: bool,
    /// `vim_clipboard = "unnamed"`: the unnamed register is the system
    /// clipboard (`"*`).
    clip_unnamed: bool,
    /// The system clipboard's text, handed in by the app before a key that
    /// may read it ([`EditorState::vim_set_clipboard`]).
    clip_in: Option<String>,
    /// Text for the system clipboard, taken by the app after the key
    /// ([`EditorState::vim_take_clipboard`]).
    clip_out: Option<String>,
    /// The last search came from `*` / `#`: it matches whole words.
    search_word: bool,
    /// `U`: the line the latest changes were on (0-based) and its text
    /// before them.
    uline: Option<(usize, String)>,
}

impl VimState {
    /// Store the text an operator removed or yanked, in the register the
    /// command named (`"a` sets, `"A` appends, `"_` drops it, `"+` / `"*`
    /// go to the system clipboard), else the unnamed register — a yank
    /// there also fills `"0`. The unnamed register always ends up holding
    /// what was stored (vim's "points to the last used register").
    fn store(&mut self, text: String, linewise: bool, yank: bool) {
        let named = self.reg_sel.take().filter(|&c| c != '"');
        // a yank into the unnamed register also fills "0, even when
        // `vim_clipboard` makes that the clipboard
        if yank && named.is_none() {
            self.regs.insert('0', (text.clone(), linewise));
        }
        let sel = named.or(self.clip_unnamed.then_some('*'));
        let (text, linewise) = match sel {
            Some('_') => return,
            Some(c) if c.is_ascii_uppercase() => {
                let key = c.to_ascii_lowercase();
                let joined = match self.regs.remove(&key) {
                    Some((old, old_lines)) if old_lines || linewise => {
                        (format!("{old}\n{text}"), true)
                    }
                    Some((old, _)) => (format!("{old}{text}"), false),
                    None => (text, linewise),
                };
                self.regs.insert(key, joined.clone());
                joined
            }
            Some(c @ ('a'..='z' | '0')) => {
                self.regs.insert(c, (text.clone(), linewise));
                (text, linewise)
            }
            Some('+' | '*') => {
                // a linewise register reaches the clipboard as whole lines
                let mut out = text.clone();
                if linewise {
                    out.push('\n');
                }
                self.clip_out = Some(out);
                (text, linewise)
            }
            _ => (text, linewise),
        };
        self.register = text;
        self.register_linewise = linewise;
    }

    /// The register a put reads: the named one, `"0`, the system clipboard
    /// (`"+` / `"*`; text ending in a newline is whole lines), else the
    /// unnamed register. `None` = nothing to put (`"_`, an empty register).
    fn fetch(&mut self) -> Option<(String, bool)> {
        let sel = self
            .reg_sel
            .take()
            .filter(|&c| c != '"')
            .or(self.clip_unnamed.then_some('*'));
        let got = match sel {
            Some('_') => None,
            Some('+' | '*') => self.clip_in.clone().map(|t| match t.strip_suffix('\n') {
                Some(lines) => (lines.to_string(), true),
                None => (t, false),
            }),
            Some(c) if c.is_ascii_alphabetic() || c == '0' => {
                self.regs.get(&c.to_ascii_lowercase()).cloned()
            }
            _ => Some((self.register.clone(), self.register_linewise)),
        };
        got.filter(|(t, _)| !t.is_empty())
    }
}

/// After a change: `U` puts back the line it was on as it was before the
/// first of the latest changes on that line. A change that added or
/// removed lines forgets it (the line numbers moved).
fn note_line_change(v: &mut VimState, now: &str) {
    if now.matches('\n').count() != v.before.matches('\n').count() {
        v.uline = None;
        return;
    }
    let at = v.mark_cursor.min(v.before.len());
    let line = v.before.get(..at).unwrap_or("").matches('\n').count();
    if v.uline.as_ref().is_some_and(|(l, _)| *l == line) {
        return;
    }
    let text = v.before.split('\n').nth(line).unwrap_or("").to_string();
    v.uline = Some((line, text));
}

/// A valid register name after `"`.
fn register_name(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '0' | '"' | '+' | '*' | '_')
}

impl VimState {
    /// In Normal mode with nothing half typed (no count, operator, pending
    /// argument or `/` prompt): the next key is a command of its own.
    pub fn normal_idle(&self) -> bool {
        self.mode == VimMode::Normal && self.pending.idle() && self.prompt.is_none()
    }

    /// Back to Insert with no half-typed command — what users expect right
    /// after running a command (matches Warp).
    pub fn reset_insert(&mut self) {
        self.mode = VimMode::Insert;
        self.pending.reset();
        self.prompt = None;
        self.insert_from = None;
        self.replaced.clear();
    }
}

fn is_visual(m: VimMode) -> bool {
    matches!(m, VimMode::Visual | VimMode::VisualLine)
}

/// Char-indexed view of the buffer: vim motions count characters, the editor
/// stores byte offsets.
struct Line {
    chars: Vec<char>,
    offs: Vec<usize>,
    len: usize,
}

impl Line {
    fn new(s: &str) -> Line {
        Line {
            chars: s.chars().collect(),
            offs: s.char_indices().map(|(i, _)| i).collect(),
            len: s.len(),
        }
    }

    fn n(&self) -> usize {
        self.chars.len()
    }

    /// Byte offset of char `i`; `i >= n` maps to the end of the buffer.
    fn byte(&self, i: usize) -> usize {
        self.offs.get(i).copied().unwrap_or(self.len)
    }

    /// Char index of a byte offset (which must be on a char boundary).
    fn index(&self, byte: usize) -> usize {
        self.offs.partition_point(|&o| o < byte)
    }

    fn last(&self) -> usize {
        self.n().saturating_sub(1)
    }

    /// Start of the line holding char `i` (a newline belongs to the line it
    /// ends).
    fn line_start(&self, i: usize) -> usize {
        let i = i.min(self.n());
        self.chars[..i]
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(0, |j| j + 1)
    }

    /// End of the line holding char `i`: its newline's index, or `n`.
    fn line_end(&self, i: usize) -> usize {
        let i = i.min(self.n());
        self.chars[i..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(self.n(), |j| i + j)
    }

    /// Start of 0-based line `k`, clamped to the last line.
    fn nth_line_start(&self, k: usize) -> usize {
        let mut start = 0;
        let mut left = k;
        for (j, &c) in self.chars.iter().enumerate() {
            if c == '\n' {
                if left == 0 {
                    break;
                }
                left -= 1;
                start = j + 1;
            }
        }
        start
    }

    /// 0-based number of the line holding char `i`.
    fn line_no(&self, i: usize) -> usize {
        self.chars[..i.min(self.n())]
            .iter()
            .filter(|&&c| c == '\n')
            .count()
    }

    /// First non-blank char of the line holding `i` (its start when blank).
    fn first_non_blank(&self, i: usize) -> usize {
        let (s, e) = (self.line_start(i), self.line_end(i));
        (s..e)
            .find(|&j| !self.chars[j].is_whitespace())
            .unwrap_or(s)
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Blank,
    Word,
    Punct,
}

/// vim word classes: `big` collapses Word/Punct so words are whitespace-delimited.
fn class(c: char, big: bool) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if big || c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// `w`: start of the next word (may return `n`, one past the last char).
fn next_word(l: &Line, mut i: usize, big: bool) -> usize {
    let n = l.n();
    if i >= n {
        return n;
    }
    let c0 = class(l.chars[i], big);
    if c0 != Class::Blank {
        while i < n && class(l.chars[i], big) == c0 {
            i += 1;
        }
    }
    while i < n && class(l.chars[i], big) == Class::Blank {
        i += 1;
    }
    i
}

/// `e`: end of the current-or-next word.
fn word_end(l: &Line, mut i: usize, big: bool) -> usize {
    let n = l.n();
    if n == 0 {
        return 0;
    }
    i += 1;
    while i < n && class(l.chars[i], big) == Class::Blank {
        i += 1;
    }
    if i >= n {
        return n - 1;
    }
    let c = class(l.chars[i], big);
    while i + 1 < n && class(l.chars[i + 1], big) == c {
        i += 1;
    }
    i
}

/// `b`: start of the previous word.
fn prev_word(l: &Line, mut i: usize, big: bool) -> usize {
    let n = l.n();
    if i == 0 || n == 0 {
        return 0;
    }
    i = i.min(n);
    i -= 1;
    while i > 0 && class(l.chars[i], big) == Class::Blank {
        i -= 1;
    }
    if class(l.chars[i], big) == Class::Blank {
        return 0;
    }
    let c = class(l.chars[i], big);
    while i > 0 && class(l.chars[i - 1], big) == c {
        i -= 1;
    }
    i
}

/// `ge`: end of the previous word (0 when there is none).
fn prev_word_end(l: &Line, i: usize, big: bool) -> usize {
    let n = l.n();
    if n == 0 {
        return 0;
    }
    let mut i = i.min(n - 1);
    let c0 = class(l.chars[i], big);
    if c0 != Class::Blank {
        while i > 0 && class(l.chars[i], big) == c0 {
            i -= 1;
        }
    }
    while i > 0 && class(l.chars[i], big) == Class::Blank {
        i -= 1;
    }
    i
}

/// `f`/`F`/`t`/`T`: the count-th `ch` from `from`, or `None` when absent.
fn find_char(l: &Line, from: usize, kind: FindKind, ch: char, count: usize) -> Option<usize> {
    let n = l.n();
    let mut hits = 0;
    match kind {
        FindKind::ForwardTo | FindKind::ForwardTill => {
            let mut i = from;
            loop {
                i += 1;
                if i >= n {
                    return None;
                }
                if l.chars[i] == ch {
                    hits += 1;
                    if hits == count {
                        return Some(if kind == FindKind::ForwardTill {
                            i - 1
                        } else {
                            i
                        });
                    }
                }
            }
        }
        FindKind::BackTo | FindKind::BackTill => {
            let mut i = from;
            loop {
                if i == 0 {
                    return None;
                }
                i -= 1;
                if l.chars[i] == ch {
                    hits += 1;
                    if hits == count {
                        return Some(if kind == FindKind::BackTill { i + 1 } else { i });
                    }
                }
            }
        }
    }
}

/// `%`: the first bracket at or after the cursor on its line, matched to
/// its partner (which may sit on another line).
fn match_bracket(l: &Line, ci: usize) -> Option<usize> {
    let i = (ci..l.line_end(ci)).find(|&j| "()[]{}".contains(l.chars[j]))?;
    let (open, close, forward) = match l.chars[i] {
        '(' => ('(', ')', true),
        ')' => ('(', ')', false),
        '[' => ('[', ']', true),
        ']' => ('[', ']', false),
        '{' => ('{', '}', true),
        _ => ('{', '}', false),
    };
    let mut depth = 0usize;
    let mut step = |j: usize, inward: char, outward: char| {
        if l.chars[j] == inward {
            depth += 1;
        } else if l.chars[j] == outward {
            if depth == 0 {
                return true;
            }
            depth -= 1;
        }
        false
    };
    if forward {
        (i + 1..l.n()).find(|&j| step(j, open, close))
    } else {
        (0..i).rev().find(|&j| step(j, close, open))
    }
}

/// `/` `?` `n` `N`: the count-th literal match of `pat` after (`forward`) or
/// before the cursor, wrapping around the buffer. `whole`: only matches
/// with no word char on either side (`*` / `#`).
fn search(
    buf: &str,
    l: &Line,
    ci: usize,
    pat: &str,
    forward: bool,
    count: usize,
    whole: bool,
) -> Option<usize> {
    if pat.is_empty() {
        return None;
    }
    let word = |c: Option<char>| c.is_some_and(|c| class(c, false) == Class::Word);
    let hits: Vec<usize> = buf
        .match_indices(pat)
        .filter(|&(b, m)| {
            !whole
                || !word(buf[..b].chars().next_back()) && !word(buf[b + m.len()..].chars().next())
        })
        .map(|(b, _)| l.index(b))
        .collect();
    let (&first, &last) = (hits.first()?, hits.last()?);
    let mut at = ci;
    for _ in 0..count {
        at = if forward {
            hits.iter().copied().find(|&h| h > at).unwrap_or(first)
        } else {
            hits.iter().copied().rfind(|&h| h < at).unwrap_or(last)
        };
    }
    Some(at)
}

/// `*` / `#`: the word (letters, digits, `_`) under char `ci`, else the next
/// one on its line, with its start.
fn word_under(l: &Line, ci: usize) -> Option<(usize, String)> {
    let is_word = |i: usize| class(l.chars[i], false) == Class::Word;
    let end_of_line = l.line_end(ci);
    let mut s = (ci..end_of_line).find(|&i| is_word(i))?;
    if s == ci {
        while s > 0 && is_word(s - 1) {
            s -= 1;
        }
    }
    let mut e = s;
    while e < l.n() && is_word(e) {
        e += 1;
    }
    Some((s, l.chars[s..e].iter().collect()))
}

/// Resolve a motion to `(target char index, inclusive)`; `None` = it failed.
fn resolve(l: &Line, ci: usize, m: Motion, count: usize) -> Option<(usize, bool)> {
    let n = l.n();
    Some(match m {
        Motion::Left => (ci.saturating_sub(count), false),
        Motion::Right => ((ci + count).min(n), false),
        Motion::LineStart => (l.line_start(ci), false),
        Motion::LineEnd => {
            let (s, e) = (l.line_start(ci), l.line_end(ci));
            // an empty line has no last char: the motion covers nothing
            if e > s { (e - 1, true) } else { (s, false) }
        }
        Motion::FirstNonBlank => (l.first_non_blank(ci), false),
        Motion::WordFwd(big) => {
            let mut i = ci;
            for _ in 0..count {
                i = next_word(l, i, big);
            }
            (i, false)
        }
        Motion::WordBack(big) => {
            let mut i = ci;
            for _ in 0..count {
                i = prev_word(l, i, big);
            }
            (i, false)
        }
        Motion::WordEnd(big) => {
            let mut i = ci;
            for _ in 0..count {
                i = word_end(l, i, big);
            }
            (i, true)
        }
        Motion::WordEndBack(big) => {
            let mut i = ci;
            for _ in 0..count {
                i = prev_word_end(l, i, big);
            }
            (i, true)
        }
        Motion::Find(kind, ch) => {
            let t = find_char(l, ci, kind, ch, count)?;
            (
                t,
                matches!(kind, FindKind::ForwardTo | FindKind::ForwardTill),
            )
        }
        Motion::GotoLine(k) => {
            let s = match k {
                Some(k) => l.nth_line_start(k.saturating_sub(1)),
                None => l.line_start(n),
            };
            (l.first_non_blank(s), false)
        }
        Motion::LineFirstNonBlank => {
            let mut s = l.line_start(ci);
            for _ in 1..count {
                let e = l.line_end(s);
                if e >= n {
                    break;
                }
                s = e + 1;
            }
            (l.first_non_blank(s), false)
        }
        Motion::LineDown | Motion::LineUp => {
            let col = ci - l.line_start(ci);
            let mut s = l.line_start(ci);
            for _ in 0..count {
                if m == Motion::LineDown {
                    let e = l.line_end(s);
                    if e >= n {
                        break;
                    }
                    s = e + 1;
                } else {
                    if s == 0 {
                        break;
                    }
                    s = l.line_start(s - 1);
                }
            }
            if s == l.line_start(ci) {
                return None;
            }
            let e = l.line_end(s);
            ((s + col).min(e.saturating_sub(1)).max(s), false)
        }
        Motion::Jump(t, inclusive) => (t, inclusive),
    })
}

/// Char range `[start, end)` of text object `obj` around char `ci`: `w` `W`
/// words, `"` `'` `` ` `` quotes, `(` `b` `[` `{` `B` `<` brackets (either
/// bracket names the pair). `inner` = the `i` form, else the `a` form (a
/// word with its trailing blanks — leading when none follow — or the
/// quotes/brackets themselves). `None` when there is no such object.
fn text_object(l: &Line, ci: usize, obj: char, inner: bool) -> Option<(usize, usize)> {
    let n = l.n();
    if n == 0 {
        return None;
    }
    let ci = ci.min(n - 1);
    let run = |from: usize, cls: Class, big: bool| {
        let mut e = from;
        while e < n && class(l.chars[e], big) == cls {
            e += 1;
        }
        e
    };
    match obj {
        'w' | 'W' => {
            let big = obj == 'W';
            let cls = class(l.chars[ci], big);
            let mut s = ci;
            while s > 0 && class(l.chars[s - 1], big) == cls {
                s -= 1;
            }
            let e = run(ci, cls, big);
            if inner {
                return Some((s, e));
            }
            if cls == Class::Blank {
                // on blanks: the blanks and the word after them
                return Some((
                    s,
                    if e < n {
                        run(e, class(l.chars[e], big), big)
                    } else {
                        e
                    },
                ));
            }
            let trailing = run(e, Class::Blank, big);
            if trailing > e {
                return Some((s, trailing));
            }
            let mut lead = s;
            while lead > 0 && l.chars[lead - 1].is_whitespace() {
                lead -= 1;
            }
            Some((lead, e))
        }
        // the cursor's line: `al` all of it (not its newline), `il`
        // without its leading and trailing blanks
        'l' => {
            let (mut s, mut e) = (l.line_start(ci), l.line_end(ci));
            if inner {
                while s < e && l.chars[s].is_whitespace() {
                    s += 1;
                }
                while e > s && l.chars[e - 1].is_whitespace() {
                    e -= 1;
                }
            }
            Some((s, e))
        }
        '"' | '\'' | '`' => {
            // quotes pair up from the line start; a backslash escapes one
            let quotes: Vec<usize> = (0..n)
                .filter(|&i| l.chars[i] == obj && (i == 0 || l.chars[i - 1] != '\\'))
                .collect();
            let pairs = || quotes.as_chunks::<2>().0.iter().map(|&[o, c]| (o, c));
            // the pair around the cursor, else the next one after it
            let (open, close) = pairs()
                .find(|&(o, c)| o <= ci && ci <= c)
                .or_else(|| pairs().find(|&(o, _)| o > ci))?;
            Some(if inner {
                (open + 1, close)
            } else {
                (open, close + 1)
            })
        }
        _ => {
            let (open_ch, close_ch) = match obj {
                '(' | ')' | 'b' => ('(', ')'),
                '[' | ']' => ('[', ']'),
                '{' | '}' | 'B' => ('{', '}'),
                '<' | '>' => ('<', '>'),
                _ => return None,
            };
            // innermost unmatched opening bracket at or left of the cursor
            // (a closing one under the cursor names its own pair)
            let mut depth = 0usize;
            let mut open = None;
            for i in (0..=ci).rev() {
                let ch = l.chars[i];
                if ch == close_ch && i != ci {
                    depth += 1;
                } else if ch == open_ch {
                    if depth == 0 {
                        open = Some(i);
                        break;
                    }
                    depth -= 1;
                }
            }
            let open = open?;
            let mut depth = 0usize;
            let close = (open + 1..n).find(|&j| {
                let ch = l.chars[j];
                if ch == open_ch {
                    depth += 1;
                } else if ch == close_ch {
                    if depth == 0 {
                        return true;
                    }
                    depth -= 1;
                }
                false
            })?;
            Some(if inner {
                (open + 1, close)
            } else {
                (open, close + 1)
            })
        }
    }
}

fn toggle_case(c: char) -> String {
    if c.is_lowercase() {
        c.to_uppercase().collect()
    } else if c.is_uppercase() {
        c.to_lowercase().collect()
    } else {
        c.to_string()
    }
}

/// The text an Insert session typed: `now` is `before` with one run inserted
/// at `at` (auto-paired brackets included). Empty when the session did
/// anything else (deleted text before it, recalled history).
fn inserted_text(before: &str, at: usize, now: &str) -> String {
    let Some(k) = now.len().checked_sub(before.len()) else {
        return String::new();
    };
    match (
        now.get(..at),
        before.get(..at),
        now.get(at + k..),
        before.get(at..),
        now.get(at..at + k),
    ) {
        (Some(a), Some(b), Some(c), Some(d), Some(typed)) if a == b && c == d => typed.to_string(),
        _ => String::new(),
    }
}

impl EditorState {
    /// Feed one key to the vim layer. Returns [`VimOutcome::PassThrough`] when
    /// vim is disabled, or in Insert mode for everything except Escape.
    pub fn vim_key(&mut self, key: VimKey) -> VimOutcome {
        let Some(mut v) = self.vim.take() else {
            return VimOutcome::PassThrough;
        };
        let out = self.vim_step(&mut v, key);
        self.vim = Some(v);
        out
    }

    /// Before a key: `vim_clipboard = "unnamed"` (`unnamed`), and the system
    /// clipboard's text when the key may read it (a put); `None` otherwise.
    pub fn vim_set_clipboard(&mut self, unnamed: bool, text: Option<String>) {
        if let Some(v) = self.vim.as_mut() {
            v.clip_unnamed = unnamed;
            v.clip_in = text;
        }
    }

    /// After a key: text vim wrote to `"+` / `"*` (or the unnamed register
    /// under `vim_clipboard = "unnamed"`), for the system clipboard.
    pub fn vim_take_clipboard(&mut self) -> Option<String> {
        let v = self.vim.as_mut()?;
        v.clip_in = None;
        v.clip_out.take()
    }

    /// Whether `key` may read the system clipboard, outside Insert mode: a
    /// put (`p` `P`) from `"+` / `"*`, or from any register but a named one
    /// under `vim_clipboard = "unnamed"` (`unnamed`), or a `.` that may
    /// repeat such a put. A put from the unnamed register never reads it.
    pub fn vim_key_reads_clipboard(&self, key: VimKey, unnamed: bool) -> bool {
        let Some(v) = self.vim.as_ref().filter(|v| v.mode != VimMode::Insert) else {
            return false;
        };
        let clip = |reg: Option<char>| match reg {
            Some('+' | '*') => true,
            None | Some('"') => unnamed,
            _ => false,
        };
        match key {
            VimKey::Char('p' | 'P') => clip(v.reg_sel),
            VimKey::Char('.') => v.last_change.as_ref().is_some_and(|c| {
                let put = c.keys.iter().any(|k| matches!(k, VimKey::Char('p' | 'P')));
                let named = c.keys.windows(2).find_map(|w| match w {
                    [VimKey::Char('"'), VimKey::Char(r)] => Some(*r),
                    _ => None,
                });
                put && clip(named)
            }),
            _ => false,
        }
    }

    /// The open `/` or `?` search prompt as typed (`/foo`), for the mode pill.
    pub fn vim_prompt(&self) -> Option<String> {
        let (forward, text) = self.vim.as_ref()?.prompt.as_ref()?;
        Some(format!("{}{text}", if *forward { '/' } else { '?' }))
    }

    /// The visual-mode selection: the inclusive char range between the
    /// anchor and the cursor (`v`), or the whole lines it touches (`V`).
    /// `None` outside the visual modes or when it is empty.
    pub(crate) fn vim_visual_range(&self, mode: VimMode) -> Option<Range<usize>> {
        let a = self.anchor?.min(self.buffer.len());
        let l = Line::new(&self.buffer);
        let (lo, hi) = (l.index(a.min(self.cursor)), l.index(a.max(self.cursor)));
        let r = match mode {
            VimMode::Visual => l.byte(lo)..l.byte(hi + 1),
            VimMode::VisualLine => l.byte(l.line_start(lo))..l.byte(l.line_end(hi)),
            _ => return None,
        };
        (!r.is_empty()).then_some(r)
    }

    /// One key, with the bookkeeping for `.` and undo around it: a command
    /// starts in Normal mode with nothing pending and ends when it gets
    /// back there (or when its Insert session ends).
    fn vim_step(&mut self, v: &mut VimState, key: VimKey) -> VimOutcome {
        if v.mode == VimMode::Insert {
            if key != VimKey::Escape {
                return VimOutcome::PassThrough;
            }
            self.leave_insert(v);
            return VimOutcome::Consumed;
        }
        // a selection cleared under visual mode (a click, a paste, undo)
        // ends it
        if is_visual(v.mode) && self.anchor.is_none() {
            v.mode = VimMode::Normal;
            v.pending.reset();
        }
        let record = !v.replaying;
        let at_rest = |v: &VimState| {
            v.mode == VimMode::Normal
                && v.pending.idle()
                && v.prompt.is_none()
                && v.reg_sel.is_none()
        };
        if record && at_rest(v) {
            v.rec.clear();
            v.before = self.buffer.clone();
            v.mark = self.undo_len();
            v.mark_cursor = self.cursor;
            v.no_record = false;
        }
        if record {
            v.rec.push(key);
        }
        let out = self.dispatch(v, key);
        // a named register lasts until the command that uses it ends: a key
        // that left nothing pending (and did not name it) drops it
        if v.reg_fresh {
            v.reg_fresh = false;
        } else if v.pending.idle() && v.prompt.is_none() {
            v.reg_sel = None;
        }
        if v.mode == VimMode::Insert && v.insert_from.is_none() {
            v.insert_from = Some((self.buffer.clone(), self.cursor));
        }
        if record && at_rest(v) {
            self.squash_undo(v.mark, v.mark_cursor);
            if !v.no_record && self.buffer != v.before {
                note_line_change(v, &self.buffer);
                v.last_change = Some(Change {
                    keys: std::mem::take(&mut v.rec),
                    insert: String::new(),
                });
            }
        }
        out
    }

    /// Esc in Insert mode: close the session (one undo step, and the last
    /// change when vim opened it) and step left like vim.
    fn leave_insert(&mut self, v: &mut VimState) {
        v.mode = VimMode::Normal;
        v.pending.reset();
        if let Some((buf, at)) = v.insert_from.take()
            && !v.replaying
        {
            self.squash_undo(v.mark, v.mark_cursor);
            if self.buffer != v.before {
                note_line_change(v, &self.buffer);
                v.last_change = Some(Change {
                    keys: std::mem::take(&mut v.rec),
                    insert: inserted_text(&buf, at, &self.buffer),
                });
            }
        }
        // vim steps left on leaving Insert (clamped at column 0)
        let l = Line::new(&self.buffer);
        let ci = l.index(self.cursor).saturating_sub(1);
        self.cursor = l.byte(ci);
        self.vim_clamp();
    }

    fn dispatch(&mut self, v: &mut VimState, key: VimKey) -> VimOutcome {
        if v.mode == VimMode::Replace {
            return self.replace_key(v, key);
        }

        // The buffer may have moved under us (history recall, undo), so
        // re-establish the on-a-char invariant every keystroke.
        self.vim_clamp();

        if v.prompt.is_some() {
            return self.prompt_key(v, key);
        }

        if let Some(aw) = v.pending.awaiting.take() {
            let VimKey::Char(c) = key else {
                v.pending.reset();
                return VimOutcome::Consumed;
            };
            return self.char_arg(v, aw, c);
        }

        let visual = is_visual(v.mode);
        match key {
            VimKey::Escape => {
                v.pending.reset();
                if visual {
                    self.leave_visual(v);
                }
                VimOutcome::Consumed
            }
            VimKey::Enter => {
                v.pending.reset();
                if visual {
                    self.leave_visual(v);
                }
                VimOutcome::Submit
            }
            VimKey::Up if visual => self.motion(v, Motion::LineUp),
            VimKey::Down if visual => self.motion(v, Motion::LineDown),
            VimKey::Up => {
                v.pending.reset();
                VimOutcome::HistoryUp
            }
            VimKey::Down => {
                v.pending.reset();
                VimOutcome::HistoryDown
            }
            VimKey::Left | VimKey::Backspace => self.motion(v, Motion::Left),
            VimKey::Right => self.motion(v, Motion::Right),
            VimKey::Redo => {
                let count = v.pending.total();
                v.pending.reset();
                v.no_record = true;
                if visual {
                    self.leave_visual(v);
                }
                for _ in 0..count {
                    self.redo();
                }
                self.vim_clamp();
                VimOutcome::Consumed
            }
            VimKey::Char(c) => self.normal_char(v, c),
        }
    }

    /// Normal mode: the cursor sits on a char, never past the last one.
    fn vim_clamp(&mut self) {
        let l = Line::new(&self.buffer);
        if l.n() == 0 {
            self.cursor = 0;
            return;
        }
        let ci = l.index(self.cursor).min(l.last());
        self.cursor = l.byte(ci);
    }

    fn leave_visual(&mut self, v: &mut VimState) {
        v.mode = VimMode::Normal;
        self.anchor = None;
    }

    /// Apply a motion: move the cursor, or feed the pending operator.
    fn motion(&mut self, v: &mut VimState, m: Motion) -> VimOutcome {
        let count = v.pending.total();
        let op = v.pending.op;
        let l = Line::new(&self.buffer);
        let ci = l.index(self.cursor);
        let n = l.n();

        // vim special case: `cw` on a non-blank behaves like `ce`, except
        // that when the cursor already sits on the last char of its word the
        // first word is just that char (it does not jump to the next word).
        let mut special = None;
        if op == Some(Op::Change)
            && let Motion::WordFwd(big) = m
            && ci < n
            && !l.chars[ci].is_whitespace()
        {
            let mut i = ci;
            for k in 0..count {
                let at_end = i + 1 >= n || class(l.chars[i + 1], big) != class(l.chars[i], big);
                if !(k == 0 && at_end) {
                    i = word_end(&l, i, big);
                }
            }
            special = Some((i, true));
        }

        let Some((t, inclusive)) = special.or_else(|| resolve(&l, ci, m, count)) else {
            v.pending.reset();
            return VimOutcome::Consumed;
        };

        match op {
            None => {
                self.cursor = if n == 0 { 0 } else { l.byte(t.min(n - 1)) };
            }
            Some(op) if m.linewise() || op.linewise() => {
                self.apply_lines(v, op, ci.min(t), ci.max(t));
            }
            Some(op) => {
                let (s, e) = if t >= ci {
                    (ci, if inclusive { (t + 1).min(n) } else { t })
                } else {
                    (t, if inclusive { (ci + 1).min(n) } else { ci })
                };
                self.apply_op(v, op, l.byte(s)..l.byte(e), false);
            }
        }
        v.pending.reset();
        VimOutcome::Consumed
    }

    /// Run an operator over a byte range.
    fn apply_op(&mut self, v: &mut VimState, op: Op, range: Range<usize>, linewise: bool) {
        if !range.is_empty() && matches!(op, Op::Delete | Op::Change | Op::Yank) {
            v.store(
                self.buffer[range.clone()].to_string(),
                linewise,
                op == Op::Yank,
            );
        }
        match op {
            Op::Yank => {
                if !range.is_empty() {
                    self.cursor = range.start;
                }
            }
            Op::Delete => {
                if range.is_empty() {
                    return;
                }
                self.replace_range(range, "");
                self.vim_clamp();
            }
            Op::Change => {
                if range.is_empty() {
                    self.cursor = range.start.min(self.buffer.len());
                } else {
                    self.replace_range(range, "");
                }
                v.mode = VimMode::Insert;
            }
            Op::Lower | Op::Upper | Op::ToggleCase => {
                let text = &self.buffer[range.clone()];
                let rep: String = match op {
                    Op::Lower => text.to_lowercase(),
                    Op::Upper => text.to_uppercase(),
                    _ => text.chars().map(toggle_case).collect(),
                };
                if rep != text {
                    self.replace_range(range.clone(), &rep);
                }
                self.cursor = range.start;
                self.vim_clamp();
            }
            Op::Indent | Op::Outdent => {
                let l = Line::new(&self.buffer);
                let (s, e) = (l.index(range.start), l.index(range.end));
                self.apply_lines(v, op, s, e.saturating_sub(1).max(s));
            }
        }
    }

    /// Run an operator over the whole lines holding chars `lo..=hi`.
    fn apply_lines(&mut self, v: &mut VimState, op: Op, lo: usize, hi: usize) {
        let l = Line::new(&self.buffer);
        let n = l.n();
        let (s, e) = (l.line_start(lo), l.line_end(hi));
        let content = l.byte(s)..l.byte(e);
        match op {
            Op::Delete => {
                v.store(self.buffer[content.clone()].to_string(), true, false);
                // the lines leave with one newline: the one after them, else
                // the one before
                let range = if e < n {
                    l.byte(s)..l.byte(e + 1)
                } else if s > 0 {
                    l.byte(s - 1)..l.byte(e)
                } else {
                    content
                };
                if !range.is_empty() {
                    self.replace_range(range, "");
                }
                let l2 = Line::new(&self.buffer);
                self.cursor = l2.byte(l2.first_non_blank(s.min(l2.n())));
                self.vim_clamp();
            }
            Op::Indent | Op::Outdent => {
                let old = &self.buffer[content.clone()];
                let new = old
                    .split('\n')
                    .map(|line| {
                        if op == Op::Indent {
                            if line.is_empty() {
                                line.to_string()
                            } else {
                                format!("  {line}")
                            }
                        } else {
                            line.strip_prefix('\t')
                                .or_else(|| line.strip_prefix("  "))
                                .or_else(|| line.strip_prefix(' '))
                                .unwrap_or(line)
                                .to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if new != old {
                    self.replace_range(content, &new);
                }
                let l2 = Line::new(&self.buffer);
                self.cursor = l2.byte(l2.first_non_blank(s));
                self.vim_clamp();
            }
            _ => self.apply_op(v, op, content, true),
        }
    }

    /// An operator key in Normal mode: start it, or run its doubled form on
    /// lines (`dd` / `cc` / `yy` take the whole buffer; `>>` `<<` `guu`
    /// `gUU` `g~~` take count lines from the cursor's).
    fn start_op(&mut self, v: &mut VimState, op: Op) -> VimOutcome {
        match v.pending.op {
            None => v.pending.op = Some(op),
            Some(prev) if prev == op => {
                let count = v.pending.total();
                v.pending.reset();
                if matches!(op, Op::Delete | Op::Change | Op::Yank) {
                    self.apply_op(v, op, 0..self.buffer.len(), true);
                } else {
                    let l = Line::new(&self.buffer);
                    let ci = l.index(self.cursor);
                    let mut last = l.line_start(ci);
                    for _ in 1..count {
                        let e = l.line_end(last);
                        if e >= l.n() {
                            break;
                        }
                        last = e + 1;
                    }
                    self.apply_lines(v, op, ci, last);
                }
                if v.mode == VimMode::Normal {
                    self.vim_clamp();
                }
            }
            Some(_) => v.pending.reset(),
        }
        VimOutcome::Consumed
    }

    /// An operator key in a visual mode: run it on the selection and leave
    /// visual mode. `lines` forces the linewise form (`D` `X` `Y` `C` `S` `R`).
    fn visual_op(&mut self, v: &mut VimState, op: Op, lines: bool) {
        let lines = lines || v.mode == VimMode::VisualLine || op.linewise();
        let Some(a) = self.anchor else {
            self.leave_visual(v);
            return;
        };
        let l = Line::new(&self.buffer);
        let (a, c) = (l.index(a.min(self.buffer.len())), l.index(self.cursor));
        let (lo, hi) = (a.min(c), a.max(c));
        self.leave_visual(v);
        if lines {
            self.apply_lines(v, op, lo, hi);
        } else {
            self.apply_op(v, op, l.byte(lo)..l.byte(hi + 1), false);
        }
        if v.mode == VimMode::Normal {
            self.vim_clamp();
        }
    }

    /// Keys that mean something else in the visual modes; `None` = handle it
    /// as in Normal mode (counts, motions, `g`, `i` / `a` objects).
    fn visual_char(&mut self, v: &mut VimState, c: char) -> Option<VimOutcome> {
        let (op, lines) = match c {
            'd' | 'x' => (Op::Delete, false),
            'X' | 'D' => (Op::Delete, true),
            'c' | 's' => (Op::Change, false),
            'C' | 'S' | 'R' => (Op::Change, true),
            'y' => (Op::Yank, false),
            'Y' => (Op::Yank, true),
            '~' => (Op::ToggleCase, false),
            'u' => (Op::Lower, false),
            'U' => (Op::Upper, false),
            '>' => (Op::Indent, true),
            '<' => (Op::Outdent, true),
            'o' => {
                // swap ends: the cursor jumps to the other end
                if let Some(a) = self.anchor {
                    self.anchor = Some(self.cursor);
                    self.cursor = a.min(self.buffer.len());
                    self.vim_clamp();
                }
                v.pending.reset();
                return Some(VimOutcome::Consumed);
            }
            'J' => {
                let l = Line::new(&self.buffer);
                let a = l.index(self.anchor.unwrap_or(self.cursor).min(self.buffer.len()));
                let ci = l.index(self.cursor);
                let (lo, hi) = (a.min(ci), a.max(ci));
                let joins = l.chars[l.line_start(lo)..l.line_end(hi)]
                    .iter()
                    .filter(|&&ch| ch == '\n')
                    .count();
                self.leave_visual(v);
                v.pending.reset();
                self.join_lines(lo, joins.max(1));
                return Some(VimOutcome::Consumed);
            }
            'v' | 'V' => {
                let want = if c == 'v' {
                    VimMode::Visual
                } else {
                    VimMode::VisualLine
                };
                if v.mode == want {
                    self.leave_visual(v);
                } else {
                    v.mode = want;
                }
                v.pending.reset();
                return Some(VimOutcome::Consumed);
            }
            _ => return None,
        };
        v.pending.reset();
        self.visual_op(v, op, lines);
        Some(VimOutcome::Consumed)
    }

    /// `J`: join `joins` lines onto the line holding char `at`, one space
    /// between (none before `)` or next to an empty line or trailing blank);
    /// the cursor lands on the last join point.
    fn join_lines(&mut self, at: usize, joins: usize) {
        for _ in 0..joins {
            let l = Line::new(&self.buffer);
            let n = l.n();
            let e = l.line_end(at);
            if e >= n {
                break;
            }
            let mut k = e + 1;
            while k < n && matches!(l.chars[k], ' ' | '\t') {
                k += 1;
            }
            let next_empty = k >= n || l.chars[k] == '\n';
            let ends_blank = e > l.line_start(at) && l.chars[e - 1].is_whitespace();
            let sep = if next_empty || ends_blank || l.chars[k] == ')' {
                ""
            } else {
                " "
            };
            self.replace_range(l.byte(e)..l.byte(k), sep);
            self.cursor = l.byte(e);
        }
        self.vim_clamp();
    }

    /// A key while the `/` / `?` prompt is open: Enter searches (an empty
    /// pattern reuses the last one), Esc — or Backspace on an empty prompt —
    /// cancels, anything else is typed.
    fn prompt_key(&mut self, v: &mut VimState, key: VimKey) -> VimOutcome {
        let Some((forward, mut text)) = v.prompt.take() else {
            return VimOutcome::Consumed;
        };
        match key {
            VimKey::Char(c) => {
                text.push(c);
                v.prompt = Some((forward, text));
            }
            VimKey::Backspace => {
                if crate::state::pop_grapheme(&mut text) {
                    v.prompt = Some((forward, text));
                } else {
                    v.pending.reset();
                }
            }
            VimKey::Enter => {
                let pat = if text.is_empty() {
                    v.last_search.take().map(|(p, _)| p)
                } else {
                    Some(text)
                };
                let Some(pat) = pat else {
                    v.pending.reset();
                    return VimOutcome::Consumed;
                };
                v.last_search = Some((pat, forward));
                v.search_word = false;
                return self.search_next(v, false);
            }
            VimKey::Escape => v.pending.reset(),
            _ => v.prompt = Some((forward, text)),
        }
        VimOutcome::Consumed
    }

    /// `n` (or Enter in the prompt) / `N` (`reverse`): jump to the next match
    /// of the last search, as an exclusive motion.
    fn search_next(&mut self, v: &mut VimState, reverse: bool) -> VimOutcome {
        let Some((pat, forward)) = v.last_search.clone() else {
            v.pending.reset();
            return VimOutcome::Consumed;
        };
        let l = Line::new(&self.buffer);
        let ci = l.index(self.cursor);
        match search(
            &self.buffer,
            &l,
            ci,
            &pat,
            forward != reverse,
            v.pending.total(),
            v.search_word,
        ) {
            Some(t) => self.motion(v, Motion::Jump(t, false)),
            None => {
                v.pending.reset();
                VimOutcome::Consumed
            }
        }
    }

    /// A key in Replace mode (`R`): chars overwrite (append at the end),
    /// Backspace restores what the last one overwrote, Esc leaves.
    fn replace_key(&mut self, v: &mut VimState, key: VimKey) -> VimOutcome {
        let l = Line::new(&self.buffer);
        let ci = l.index(self.cursor);
        match key {
            VimKey::Char(c) => {
                let s = c.to_string();
                if ci < l.n() && l.chars[ci] != '\n' {
                    v.replaced.push(Some(l.chars[ci]));
                    self.replace_range(l.byte(ci)..l.byte(ci + 1), &s);
                } else {
                    v.replaced.push(None);
                    self.replace_range(self.cursor..self.cursor, &s);
                }
            }
            VimKey::Backspace => {
                if ci > 0 {
                    let at = l.byte(ci - 1)..l.byte(ci);
                    match v.replaced.pop() {
                        Some(Some(orig)) => {
                            self.replace_range(at.clone(), &orig.to_string());
                            self.cursor = at.start;
                        }
                        Some(None) => self.replace_range(at, ""),
                        None => self.cursor = at.start,
                    }
                }
            }
            VimKey::Left => {
                v.replaced.clear();
                self.cursor = l.byte(ci.saturating_sub(1));
            }
            VimKey::Right => {
                v.replaced.clear();
                self.cursor = l.byte((ci + 1).min(l.n()));
            }
            VimKey::Escape => {
                v.replaced.clear();
                v.mode = VimMode::Normal;
                self.cursor = l.byte(ci.saturating_sub(1));
                self.vim_clamp();
            }
            VimKey::Enter => return VimOutcome::Submit,
            VimKey::Up => return VimOutcome::HistoryUp,
            VimKey::Down => return VimOutcome::HistoryDown,
            VimKey::Redo => {}
        }
        VimOutcome::Consumed
    }

    /// The char argument of `r` / `f` / `F` / `t` / `T` / `g` / a text object.
    fn char_arg(&mut self, v: &mut VimState, aw: Awaiting, c: char) -> VimOutcome {
        match aw {
            Awaiting::Object(inner) => {
                let op = v.pending.op;
                v.pending.reset();
                let l = Line::new(&self.buffer);
                let obj = text_object(&l, l.index(self.cursor), c, inner);
                if is_visual(v.mode) {
                    // `viw`: select the object (charwise)
                    if let Some((s, e)) = obj
                        && e > s
                    {
                        v.mode = VimMode::Visual;
                        self.anchor = Some(l.byte(s));
                        self.cursor = l.byte(e - 1);
                    }
                } else if let (Some(op), Some((s, e))) = (op, obj) {
                    self.apply_op(v, op, l.byte(s)..l.byte(e), false);
                }
                VimOutcome::Consumed
            }
            Awaiting::Register => {
                if register_name(c) {
                    v.reg_sel = Some(c);
                    v.reg_fresh = true;
                } else {
                    v.pending.reset();
                }
                VimOutcome::Consumed
            }
            Awaiting::Find(kind) => {
                v.last_find = Some((kind, c));
                self.motion(v, Motion::Find(kind, c))
            }
            Awaiting::G => match c {
                'g' => {
                    let line = v.pending.given().unwrap_or(1);
                    self.motion(v, Motion::GotoLine(Some(line)))
                }
                'e' | 'E' => self.motion(v, Motion::WordEndBack(c == 'E')),
                'u' | 'U' | '~' => {
                    let op = match c {
                        'u' => Op::Lower,
                        'U' => Op::Upper,
                        _ => Op::ToggleCase,
                    };
                    if is_visual(v.mode) {
                        v.pending.reset();
                        self.visual_op(v, op, false);
                        VimOutcome::Consumed
                    } else {
                        self.start_op(v, op)
                    }
                }
                _ => {
                    v.pending.reset();
                    VimOutcome::Consumed
                }
            },
            Awaiting::Replace => {
                let count = v.pending.total();
                v.pending.reset();
                let l = Line::new(&self.buffer);
                let ci = l.index(self.cursor);
                // vim aborts when the line is too short for the count
                if ci + count > l.n() {
                    return VimOutcome::Consumed;
                }
                let rep = c.to_string().repeat(count);
                self.replace_range(l.byte(ci)..l.byte(ci + count), &rep);
                // cursor lands on the last replaced char
                let l2 = Line::new(&self.buffer);
                self.cursor = l2.byte(ci + count - 1);
                self.vim_clamp();
                VimOutcome::Consumed
            }
        }
    }

    fn normal_char(&mut self, v: &mut VimState, c: char) -> VimOutcome {
        // counts: 1-9 start one, 0 only extends one (bare `0` is a motion)
        if let Some(d) = c.to_digit(10) {
            let slot = if v.pending.op.is_some() {
                v.pending.op_count
            } else {
                v.pending.count
            };
            if d != 0 || slot.is_some() {
                let next = slot
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(d as usize)
                    .min(COUNT_CAP);
                if v.pending.op.is_some() {
                    v.pending.op_count = Some(next);
                } else {
                    v.pending.count = Some(next);
                }
                return VimOutcome::Consumed;
            }
        }

        // `"x`: the register the next operator or put uses
        if c == '"' && v.pending.op.is_none() {
            v.pending.awaiting = Some(Awaiting::Register);
            return VimOutcome::Consumed;
        }

        let visual = is_visual(v.mode);
        if visual && let Some(out) = self.visual_char(v, c) {
            return out;
        }

        // operators; a doubled operator is the linewise form (dd / cc / yy,
        // >> / <<, guu / gUU / g~~)
        if let Some(op) = v.pending.op
            && op.case_key() == Some(c)
        {
            return self.start_op(v, op);
        }
        let as_op = match c {
            'd' => Some(Op::Delete),
            'c' => Some(Op::Change),
            'y' => Some(Op::Yank),
            '>' => Some(Op::Indent),
            '<' => Some(Op::Outdent),
            _ => None,
        };
        if let Some(op) = as_op {
            return self.start_op(v, op);
        }

        let motion = match c {
            'h' => Some(Motion::Left),
            'l' | ' ' => Some(Motion::Right),
            '0' => Some(Motion::LineStart),
            '$' => Some(Motion::LineEnd),
            '^' => Some(Motion::FirstNonBlank),
            '_' => Some(Motion::LineFirstNonBlank),
            'w' => Some(Motion::WordFwd(false)),
            'W' => Some(Motion::WordFwd(true)),
            'b' => Some(Motion::WordBack(false)),
            'B' => Some(Motion::WordBack(true)),
            'e' => Some(Motion::WordEnd(false)),
            'E' => Some(Motion::WordEnd(true)),
            'G' => Some(Motion::GotoLine(v.pending.given())),
            'j' if visual => Some(Motion::LineDown),
            'k' if visual => Some(Motion::LineUp),
            _ => None,
        };
        if let Some(m) = motion {
            return self.motion(v, m);
        }

        match c {
            'g' => {
                v.pending.awaiting = Some(Awaiting::G);
                return VimOutcome::Consumed;
            }
            '%' => {
                let l = Line::new(&self.buffer);
                return match match_bracket(&l, l.index(self.cursor)) {
                    Some(t) => self.motion(v, Motion::Jump(t, true)),
                    None => {
                        v.pending.reset();
                        VimOutcome::Consumed
                    }
                };
            }
            '/' | '?' => {
                v.prompt = Some((c == '/', String::new()));
                return VimOutcome::Consumed;
            }
            'n' | 'N' => return self.search_next(v, c == 'N'),
            // the word under the cursor (else the next one on its line),
            // as a whole word, forward / backward
            '*' | '#' => {
                let l = Line::new(&self.buffer);
                let Some((start, word)) = word_under(&l, l.index(self.cursor)) else {
                    v.pending.reset();
                    return VimOutcome::Consumed;
                };
                // searched from the word's start, so `#` passes the word
                // the cursor is in (vim)
                if v.pending.op.is_none() {
                    self.cursor = l.byte(start);
                }
                v.last_search = Some((word, c == '*'));
                v.search_word = true;
                return self.search_next(v, false);
            }
            _ => {}
        }

        let find = match c {
            'f' => Some(FindKind::ForwardTo),
            'F' => Some(FindKind::BackTo),
            't' => Some(FindKind::ForwardTill),
            'T' => Some(FindKind::BackTill),
            _ => None,
        };
        if let Some(k) = find {
            v.pending.awaiting = Some(Awaiting::Find(k));
            return VimOutcome::Consumed;
        }
        if c == ';' || c == ',' {
            return match v.last_find {
                Some((k, ch)) => {
                    let k = if c == ',' { k.reversed() } else { k };
                    self.motion(v, Motion::Find(k, ch))
                }
                None => {
                    v.pending.reset();
                    VimOutcome::Consumed
                }
            };
        }

        // a text object (`ciw`, `da"`, `yi(`, `viw`)
        if (v.pending.op.is_some() || visual) && matches!(c, 'i' | 'a') {
            v.pending.awaiting = Some(Awaiting::Object(c == 'i'));
            return VimOutcome::Consumed;
        }

        // anything else while an operator is pending cancels it; other
        // keys mean nothing in the visual modes
        if v.pending.op.is_some() || visual {
            v.pending.reset();
            return VimOutcome::Consumed;
        }

        let given = v.pending.count;
        let count = v.pending.total();
        v.pending.reset();
        let l = Line::new(&self.buffer);
        let ci = l.index(self.cursor);
        let n = l.n();
        match c {
            'i' => v.mode = VimMode::Insert,
            'a' => {
                self.cursor = l.byte((ci + 1).min(n));
                v.mode = VimMode::Insert;
            }
            'I' => {
                self.cursor = l.byte(l.first_non_blank(ci));
                v.mode = VimMode::Insert;
            }
            'A' => {
                self.cursor = l.byte(l.line_end(ci));
                v.mode = VimMode::Insert;
            }
            'o' => {
                let at = l.byte(l.line_end(ci));
                self.replace_range(at..at, "\n");
                v.mode = VimMode::Insert;
            }
            'O' => {
                let at = l.byte(l.line_start(ci));
                self.replace_range(at..at, "\n");
                self.cursor = at;
                v.mode = VimMode::Insert;
            }
            'v' | 'V' => {
                v.mode = if c == 'v' {
                    VimMode::Visual
                } else {
                    VimMode::VisualLine
                };
                self.anchor = Some(self.cursor);
            }
            'R' => {
                v.replaced.clear();
                v.mode = VimMode::Replace;
            }
            'J' => self.join_lines(ci, count.max(2) - 1),
            'x' => {
                let e = (ci + count).min(n);
                self.apply_op(v, Op::Delete, l.byte(ci)..l.byte(e), false);
            }
            'X' => {
                let s = ci.saturating_sub(count);
                self.apply_op(v, Op::Delete, l.byte(s)..l.byte(ci), false);
            }
            's' => {
                let e = (ci + count).min(n);
                self.apply_op(v, Op::Change, l.byte(ci)..l.byte(e), false);
            }
            'S' => self.apply_op(v, Op::Change, 0..self.buffer.len(), true),
            // to the end of the cursor's line, not of the command
            'C' => self.apply_op(v, Op::Change, self.cursor..l.byte(l.line_end(ci)), false),
            'D' => self.apply_op(v, Op::Delete, self.cursor..l.byte(l.line_end(ci)), false),
            'p' => self.put(v, count, true),
            'P' => self.put(v, count, false),
            'r' => {
                v.pending.count = Some(count);
                v.pending.awaiting = Some(Awaiting::Replace);
            }
            '~' => {
                let e = (ci + count).min(n);
                if e > ci {
                    let rep: String = l.chars[ci..e].iter().map(|&ch| toggle_case(ch)).collect();
                    self.replace_range(l.byte(ci)..l.byte(e), &rep);
                }
            }
            '.' => self.repeat(v, given),
            'u' => {
                v.no_record = true;
                for _ in 0..count {
                    self.undo();
                }
            }
            // line undo: the line back as it was before the latest changes
            // on it; a change itself, so a second `U` undoes the first
            'U' => {
                v.no_record = true;
                if let Some((line, text)) = v.uline.take() {
                    let l = Line::new(&self.buffer);
                    let s = l.nth_line_start(line);
                    if l.line_no(s) == line {
                        let range = l.byte(s)..l.byte(l.line_end(s));
                        let now = self.buffer[range.clone()].to_string();
                        if now != text {
                            self.replace_range(range, &text);
                        }
                        self.cursor = l.byte(s);
                        v.uline = Some((line, now));
                    }
                }
            }
            'j' => return VimOutcome::HistoryDown,
            'k' => return VimOutcome::HistoryUp,
            _ => {}
        }
        if v.mode == VimMode::Normal {
            self.vim_clamp();
        }
        VimOutcome::Consumed
    }

    /// `.`: replay the last change's keys (a count replaces its count), then
    /// the text its Insert session typed. The caller's command bookkeeping
    /// folds it all into one undo step.
    fn repeat(&mut self, v: &mut VimState, given: Option<usize>) {
        v.no_record = true;
        let Some(change) = v.last_change.clone() else {
            return;
        };
        let mut keys = change.keys;
        if let Some(count) = given
            && !matches!(keys.first(), Some(VimKey::Char('v' | 'V')))
        {
            let typed = keys
                .iter()
                .take_while(|k| matches!(k, VimKey::Char(d) if d.is_ascii_digit()))
                .count();
            keys.splice(..typed, count.to_string().chars().map(VimKey::Char));
        }
        v.replaying = true;
        for k in keys {
            self.vim_step(v, k);
        }
        if v.mode == VimMode::Insert {
            if !change.insert.is_empty() {
                self.insert(&change.insert);
            }
            self.vim_step(v, VimKey::Escape);
        }
        v.replaying = false;
    }

    /// `p` / `P`. A linewise register has no second line to land on here, so
    /// it goes at the end (`p`) or the start (`P`) of the single line.
    fn put(&mut self, v: &mut VimState, count: usize, after: bool) {
        let Some((reg, linewise)) = v.fetch() else {
            return;
        };
        let text = reg.repeat(count);
        if linewise {
            let at = if after { self.buffer.len() } else { 0 };
            self.replace_range(at..at, &text);
            self.cursor = at;
            return;
        }
        let l = Line::new(&self.buffer);
        let ci = l.index(self.cursor);
        let at = if after {
            l.byte((ci + 1).min(l.n()))
        } else {
            l.byte(ci)
        };
        self.replace_range(at..at, &text);
        // cursor lands on the last inserted char
        let l2 = Line::new(&self.buffer);
        let end = l2.index(at + text.len());
        self.cursor = l2.byte(end.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::{VimKey, VimMode, VimOutcome, VimState};
    use crate::state::EditorState;

    /// Editor with vim enabled, `s` typed in (so: Insert, cursor at the end).
    fn ed(s: &str) -> EditorState {
        let mut e = EditorState::default();
        e.vim = Some(VimState::default());
        if !s.is_empty() {
            e.insert(s);
        }
        e
    }

    fn esc(e: &mut EditorState) -> VimOutcome {
        e.vim_key(VimKey::Escape)
    }

    fn keys(e: &mut EditorState, s: &str) -> VimOutcome {
        let mut out = VimOutcome::Consumed;
        for c in s.chars() {
            out = e.vim_key(VimKey::Char(c));
        }
        out
    }

    const GIT: &str = "git  commit -m \"msg\" --amend";

    /// `s` in Normal mode with the cursor on char `at`, then `cmd` typed.
    fn after(s: &str, at: usize, cmd: &str) -> (String, Option<VimMode>, String) {
        let e = run(s, at, cmd);
        let reg = e.vim.as_ref().unwrap().register.clone();
        (e.buffer.clone(), e.vim_mode(), reg)
    }

    /// The editor after `after`'s steps (char `at` counts from the buffer
    /// start, across lines).
    fn run(s: &str, at: usize, cmd: &str) -> EditorState {
        let mut e = ed(s);
        esc(&mut e);
        keys(&mut e, "gg0");
        for _ in 0..at {
            keys(&mut e, "l");
        }
        keys(&mut e, cmd);
        e
    }

    /// `run`, then the selected text.
    fn sel(s: &str, at: usize, cmd: &str) -> Option<String> {
        run(s, at, cmd).selected_text().map(str::to_string)
    }

    #[test]
    fn text_objects() {
        // words: inner, around (trailing blanks, else leading), WORDs
        assert_eq!(after("echo one two", 6, "diw").0, "echo  two");
        assert_eq!(after("echo one two", 6, "daw").0, "echo two");
        assert_eq!(after("echo one two", 10, "daw").0, "echo one");
        assert_eq!(
            after("cd a/b-c x", 4, "ciW"),
            ("cd  x".into(), Some(VimMode::Insert), "a/b-c".into())
        );
        assert_eq!(after("thre!", 4, "ciw").0, "thre");
        // quotes: around the cursor, else the next pair; escaped ones skipped
        assert_eq!(after(GIT, 17, "ci\"").0, "git  commit -m \"\" --amend");
        assert_eq!(after(GIT, 0, "da\"").0, "git  commit -m  --amend");
        assert_eq!(after(r#"echo "a \" b" c"#, 6, "di\"").0, r#"echo "" c"#);
        assert_eq!(after("echo 'x'", 1, "yi'").2, "x");
        // brackets: innermost pair, from either bracket
        assert_eq!(after("f(a, (b), c)", 3, "di(").0, "f()");
        assert_eq!(after("f(a, (b), c)", 6, "dib").0, "f(a, (), c)");
        assert_eq!(after("f(a, (b), c)", 11, "da)").0, "f");
        assert_eq!(after("x[1] {y}", 6, "ci{").0, "x[1] {}");
        // nothing to act on: the buffer stays, the operator is dropped
        assert_eq!(after("no brackets", 3, "di(").0, "no brackets");
        assert_eq!(after("no quotes", 3, "ci\"").1, Some(VimMode::Normal));
    }

    #[test]
    fn disabled_passes_everything_through() {
        let mut e = EditorState::default();
        assert!(!e.vim_enabled());
        assert_eq!(e.vim_mode(), None);
        assert_eq!(e.vim_key(VimKey::Escape), VimOutcome::PassThrough);
        assert_eq!(e.vim_key(VimKey::Char('x')), VimOutcome::PassThrough);
    }

    #[test]
    fn mode_switching() {
        let mut e = ed("abc");
        assert!(e.vim_enabled());
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        // insert mode passes everything but Escape through
        assert_eq!(e.vim_key(VimKey::Char('z')), VimOutcome::PassThrough);
        assert_eq!(e.buffer, "abc");

        assert_eq!(esc(&mut e), VimOutcome::Consumed);
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));
        assert_eq!(e.cursor, 2);

        keys(&mut e, "i");
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        assert_eq!(e.cursor, 2);
        esc(&mut e);
        assert_eq!(e.cursor, 1);

        keys(&mut e, "a");
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        assert_eq!(e.cursor, 2);
        esc(&mut e);

        keys(&mut e, "A");
        assert_eq!(e.cursor, 3);
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        esc(&mut e);

        keys(&mut e, "I");
        assert_eq!(e.cursor, 0);
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
    }

    #[test]
    fn escape_from_insert_moves_cursor_left() {
        let mut e = ed("echo");
        assert_eq!(e.cursor, 4);
        esc(&mut e);
        assert_eq!(e.buffer, "echo");
        assert_eq!(e.cursor, 3);
        // clamped at column 0
        keys(&mut e, "0i");
        esc(&mut e);
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn char_motions() {
        //             0123456789
        let mut e = ed("  echo hi");
        esc(&mut e);
        assert_eq!(e.cursor, 8);
        keys(&mut e, "0");
        assert_eq!(e.cursor, 0);
        keys(&mut e, "^");
        assert_eq!(e.cursor, 2);
        keys(&mut e, "l");
        assert_eq!(e.cursor, 3);
        keys(&mut e, "h");
        assert_eq!(e.cursor, 2);
        keys(&mut e, "$");
        assert_eq!(e.cursor, 8);
        // `l` on the last char does not run off the end
        keys(&mut e, "l");
        assert_eq!(e.cursor, 8);
        keys(&mut e, "0h");
        assert_eq!(e.cursor, 0);
        // arrow keys mirror h / l
        e.vim_key(VimKey::Right);
        assert_eq!(e.cursor, 1);
        e.vim_key(VimKey::Left);
        assert_eq!(e.cursor, 0);
        e.vim_key(VimKey::Backspace);
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn word_motions_respect_punctuation_classes() {
        // git  commit -m "msg" --amend
        // 0    5      12 15    21
        let mut e = ed(GIT);
        esc(&mut e);
        assert_eq!(e.cursor, 27); // last char 'd' (len 28)
        keys(&mut e, "0");
        // `-m` is punct then word, `"msg"` is quote / word / quote
        for want in [5, 12, 13, 15, 16, 19, 21, 23, 27] {
            keys(&mut e, "w");
            assert_eq!(e.cursor, want, "w landed wrong");
        }
        // `w` at the end clamps to the last char
        keys(&mut e, "w");
        assert_eq!(e.cursor, 27);
        for want in [23, 21, 19, 16, 15, 13, 12, 5, 0] {
            keys(&mut e, "b");
            assert_eq!(e.cursor, want, "b landed wrong");
        }
        for want in [2, 10, 12, 13, 15, 18, 19, 22, 27] {
            keys(&mut e, "e");
            assert_eq!(e.cursor, want, "e landed wrong");
        }
    }

    #[test]
    fn big_word_motions_are_whitespace_delimited() {
        let mut e = ed(GIT);
        esc(&mut e);
        keys(&mut e, "0");
        for want in [5, 12, 15, 21] {
            keys(&mut e, "W");
            assert_eq!(e.cursor, want, "W landed wrong");
        }
        for want in [15, 12, 5, 0] {
            keys(&mut e, "B");
            assert_eq!(e.cursor, want, "B landed wrong");
        }
        for want in [2, 10, 13, 19, 27] {
            keys(&mut e, "E");
            assert_eq!(e.cursor, want, "E landed wrong");
        }
    }

    #[test]
    fn x_and_capital_x() {
        let mut e = ed("abcd");
        esc(&mut e);
        keys(&mut e, "0x");
        assert_eq!(e.buffer, "bcd");
        assert_eq!(e.cursor, 0);
        keys(&mut e, "$X");
        assert_eq!(e.buffer, "bd");
        assert_eq!(e.cursor, 1);
        // x on the last char steps back
        keys(&mut e, "x");
        assert_eq!(e.buffer, "b");
        assert_eq!(e.cursor, 0);
        keys(&mut e, "x");
        assert_eq!(e.buffer, "");
        assert_eq!(e.cursor, 0);
        // x on an empty buffer is a no-op
        keys(&mut e, "x");
        assert_eq!(e.buffer, "");
    }

    #[test]
    fn delete_and_change_operators() {
        // dw
        let mut e = ed("git  commit -m x");
        esc(&mut e);
        keys(&mut e, "0dw");
        assert_eq!(e.buffer, "commit -m x");
        assert_eq!(e.cursor, 0);
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));

        // cw behaves like ce: it stops at the end of the word, not the space
        let mut e = ed("git commit");
        esc(&mut e);
        keys(&mut e, "0cw");
        assert_eq!(e.buffer, " commit");
        assert_eq!(e.cursor, 0);
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        e.insert("svn");
        assert_eq!(e.buffer, "svn commit");

        // cw on the last char of a word / a one-char word changes just it
        let mut e = ed("ab cd");
        esc(&mut e);
        keys(&mut e, "0lcw");
        assert_eq!(e.buffer, "a cd");
        let mut e = ed("a b c");
        esc(&mut e);
        keys(&mut e, "0cw");
        assert_eq!(e.buffer, " b c");
        let mut e = ed("a b c");
        esc(&mut e);
        keys(&mut e, "0c2w");
        assert_eq!(e.buffer, " c");

        // D / C from mid-line
        let mut e = ed("echo hi");
        esc(&mut e);
        keys(&mut e, "0wD");
        assert_eq!(e.buffer, "echo ");
        assert_eq!(e.cursor, 4);
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));

        let mut e = ed("echo hi");
        esc(&mut e);
        keys(&mut e, "0wC");
        assert_eq!(e.buffer, "echo ");
        assert_eq!(e.cursor, 5);
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));

        // D / C stop at the cursor's line end in a multi-line command
        let mut e = ed("echo one\necho two");
        esc(&mut e);
        keys(&mut e, "gg0wD");
        assert_eq!(e.buffer, "echo \necho two");
        let mut e = ed("echo one\necho two");
        esc(&mut e);
        keys(&mut e, "gg0wC");
        assert_eq!(e.buffer, "echo \necho two");
        assert_eq!(e.cursor, 5);

        // dd / cc
        let mut e = ed("foo bar");
        esc(&mut e);
        keys(&mut e, "dd");
        assert_eq!(e.buffer, "");
        assert_eq!(e.cursor, 0);
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));

        let mut e = ed("foo bar");
        esc(&mut e);
        keys(&mut e, "cc");
        assert_eq!(e.buffer, "");
        assert_eq!(e.cursor, 0);
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));

        // S is cc; s deletes the char and inserts
        let mut e = ed("foo bar");
        esc(&mut e);
        keys(&mut e, "S");
        assert_eq!(e.buffer, "");
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));

        let mut e = ed("cat");
        esc(&mut e);
        keys(&mut e, "0s");
        assert_eq!(e.buffer, "at");
        assert_eq!(e.cursor, 0);
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        e.insert("b");
        assert_eq!(e.buffer, "bat");

        // d$ and d0
        let mut e = ed("echo hi");
        esc(&mut e);
        keys(&mut e, "0wd$");
        assert_eq!(e.buffer, "echo ");
        let mut e = ed("echo hi");
        esc(&mut e);
        keys(&mut e, "0wd0");
        assert_eq!(e.buffer, "hi");
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn counts_prefix_motions_and_operators() {
        //             0   4   8     14
        let mut e = ed("one two three four");
        esc(&mut e);
        keys(&mut e, "0");
        keys(&mut e, "3w");
        assert_eq!(e.cursor, 14);

        let mut e = ed("abcdef");
        esc(&mut e);
        keys(&mut e, "02x");
        assert_eq!(e.buffer, "cdef");
        assert_eq!(e.cursor, 0);

        // count after the operator
        let mut e = ed("one two three");
        esc(&mut e);
        keys(&mut e, "0d2w");
        assert_eq!(e.buffer, "three");
        assert_eq!(e.cursor, 0);

        // count before the operator
        let mut e = ed("one two three");
        esc(&mut e);
        keys(&mut e, "02dw");
        assert_eq!(e.buffer, "three");

        // counts multiply: 2d3w would eat six words; here it eats the line
        let mut e = ed("a b c d e f g");
        esc(&mut e);
        keys(&mut e, "02d3w");
        assert_eq!(e.buffer, "g");

        // Escape cancels a half-typed operator + count
        let mut e = ed("abc");
        esc(&mut e);
        keys(&mut e, "02d");
        esc(&mut e);
        assert_eq!(e.buffer, "abc");
        keys(&mut e, "x");
        assert_eq!(e.buffer, "bc");
    }

    #[test]
    fn yank_and_put() {
        // yy + p appends the line; the register survives
        let mut e = ed("echo");
        esc(&mut e);
        keys(&mut e, "yyp");
        assert_eq!(e.buffer, "echoecho");
        assert_eq!(e.cursor, 4);

        let mut e = ed("echo");
        esc(&mut e);
        keys(&mut e, "yyP");
        assert_eq!(e.buffer, "echoecho");
        assert_eq!(e.cursor, 0);

        // dd + p restores the line
        let mut e = ed("git status");
        esc(&mut e);
        keys(&mut e, "ddp");
        assert_eq!(e.buffer, "git status");
        assert_eq!(e.cursor, 0);

        // charwise: x then p transposes
        let mut e = ed("abc");
        esc(&mut e);
        keys(&mut e, "0x");
        assert_eq!(e.buffer, "bc");
        keys(&mut e, "p");
        assert_eq!(e.buffer, "bac");
        assert_eq!(e.cursor, 1);
        keys(&mut e, "P");
        assert_eq!(e.buffer, "baac");
        assert_eq!(e.cursor, 1);

        // yw yanks a word
        let mut e = ed("one two");
        esc(&mut e);
        keys(&mut e, "0yw$p");
        assert_eq!(e.buffer, "one twoone ");
        // put with a count
        let mut e = ed("ab");
        esc(&mut e);
        keys(&mut e, "0x2p");
        assert_eq!(e.buffer, "baa");
    }

    #[test]
    fn replace_and_toggle_case() {
        let mut e = ed("cat");
        esc(&mut e);
        keys(&mut e, "0r");
        keys(&mut e, "b");
        assert_eq!(e.buffer, "bat");
        assert_eq!(e.cursor, 0);

        // count: replace three chars
        let mut e = ed("cat");
        esc(&mut e);
        keys(&mut e, "03r");
        keys(&mut e, "x");
        assert_eq!(e.buffer, "xxx");
        assert_eq!(e.cursor, 2);

        // not enough chars left: vim aborts, buffer untouched
        let mut e = ed("cat");
        esc(&mut e);
        keys(&mut e, "$2r");
        keys(&mut e, "z");
        assert_eq!(e.buffer, "cat");

        // ~ toggles and advances
        let mut e = ed("abc");
        esc(&mut e);
        keys(&mut e, "0~");
        assert_eq!(e.buffer, "Abc");
        assert_eq!(e.cursor, 1);
        keys(&mut e, "03~");
        assert_eq!(e.buffer, "aBC");
        assert_eq!(e.cursor, 2);
    }

    #[test]
    fn find_till_and_repeat() {
        //             0123456789.
        let mut e = ed("foo bar baz");
        esc(&mut e);
        keys(&mut e, "0f");
        keys(&mut e, "b");
        assert_eq!(e.cursor, 4);
        keys(&mut e, ";");
        assert_eq!(e.cursor, 8);
        keys(&mut e, "F");
        keys(&mut e, "o");
        assert_eq!(e.cursor, 2);
        keys(&mut e, "t");
        keys(&mut e, "z");
        assert_eq!(e.cursor, 9);
        keys(&mut e, "T");
        keys(&mut e, "f");
        assert_eq!(e.cursor, 1);
        // a char that isn't on the line leaves the cursor put
        keys(&mut e, "f");
        keys(&mut e, "Q");
        assert_eq!(e.cursor, 1);
        // find as an operator target (inclusive)
        keys(&mut e, "0df");
        keys(&mut e, "r");
        assert_eq!(e.buffer, " baz");
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn undo_and_redo_from_normal_mode() {
        let mut e = ed("git commit");
        esc(&mut e);
        keys(&mut e, "0dw");
        assert_eq!(e.buffer, "commit");
        keys(&mut e, "u");
        assert_eq!(e.buffer, "git commit");
        assert_eq!(e.cursor, 0);
        e.vim_key(VimKey::Redo);
        assert_eq!(e.buffer, "commit");
        assert_eq!(e.cursor, 0);
        // a count redoes that many
        keys(&mut e, "xxuu");
        assert_eq!(e.buffer, "commit");
        keys(&mut e, "2");
        e.vim_key(VimKey::Redo);
        assert_eq!(e.buffer, "mmit");
    }

    #[test]
    fn capital_u_undoes_the_line_and_itself() {
        let mut e = ed("git status");
        esc(&mut e);
        keys(&mut e, "0xx");
        keys(&mut e, "$x");
        assert_eq!(e.buffer, "t statu");
        // every change on the line, at once
        keys(&mut e, "U");
        assert_eq!(e.buffer, "git status");
        // U is a change: U again redoes them, u undoes the U
        keys(&mut e, "U");
        assert_eq!(e.buffer, "t statu");
        keys(&mut e, "u");
        assert_eq!(e.buffer, "git status");
        // a change on another line starts over there
        let mut e = ed("ab\ncd");
        esc(&mut e);
        keys(&mut e, "ggx");
        keys(&mut e, "Gx");
        assert_eq!(e.buffer, "b\nd");
        keys(&mut e, "U");
        assert_eq!(e.buffer, "b\ncd");
        // nothing changed yet: U does nothing
        let mut e = ed("abc");
        esc(&mut e);
        keys(&mut e, "U");
        assert_eq!(e.buffer, "abc");
    }

    #[test]
    fn named_registers_set_append_and_put() {
        let mut e = ed("one two three");
        esc(&mut e);
        keys(&mut e, "0\"ayw");
        keys(&mut e, "w\"byw");
        // the unnamed register holds the last one; "a kept its own
        keys(&mut e, "$\"ap");
        assert_eq!(e.buffer, "one two threeone ");
        keys(&mut e, "\"bP");
        assert_eq!(e.buffer, "one two threeonetwo  ");
        // "A appends
        let mut e = ed("ab cd");
        esc(&mut e);
        keys(&mut e, "0\"ayl");
        keys(&mut e, "w\"Ayl");
        keys(&mut e, "$\"ap");
        assert_eq!(e.buffer, "ab cdac");
        // a count after the register name
        let mut e = ed("xy");
        esc(&mut e);
        keys(&mut e, "0\"qyl\"q3p");
        assert_eq!(e.buffer, "xxxxy");
        // a register name dies with a command that does not use it
        let mut e = ed("ab");
        esc(&mut e);
        keys(&mut e, "0\"ayl");
        keys(&mut e, "\"bl");
        keys(&mut e, "x");
        // x went to the unnamed register, not "b
        keys(&mut e, "\"bp");
        assert_eq!(e.buffer, "a");
        keys(&mut e, "p");
        assert_eq!(e.buffer, "ab");
    }

    #[test]
    fn yank_register_and_black_hole() {
        // "0 keeps the last yank through a delete
        let mut e = ed("keep gone");
        esc(&mut e);
        keys(&mut e, "0yw");
        keys(&mut e, "wdw");
        assert_eq!(e.vim.as_ref().unwrap().register, "gone");
        keys(&mut e, "\"0P");
        assert_eq!(e.buffer, "keepkeep  ");
        // a yank into a named register leaves "0 alone
        let mut e = ed("one two");
        esc(&mut e);
        keys(&mut e, "0yw");
        keys(&mut e, "w\"ayw");
        keys(&mut e, "0\"0P");
        assert_eq!(e.buffer, "one one two");
        // "_ drops the text: the unnamed register keeps the yank
        let mut e = ed("abc");
        esc(&mut e);
        keys(&mut e, "0yl");
        keys(&mut e, "l\"_x");
        assert_eq!(e.buffer, "ac");
        assert_eq!(e.vim.as_ref().unwrap().register, "a");
        // "_p puts nothing
        keys(&mut e, "\"_p");
        assert_eq!(e.buffer, "ac");
    }

    #[test]
    fn clipboard_registers() {
        // "+y writes the clipboard; plain y does not
        let mut e = ed("hello world");
        esc(&mut e);
        keys(&mut e, "0\"+yw");
        assert_eq!(e.vim_take_clipboard().as_deref(), Some("hello "));
        keys(&mut e, "yw");
        assert_eq!(e.vim_take_clipboard(), None);
        // "*p reads what the app handed in; a trailing newline is linewise
        let mut e = ed("ab");
        esc(&mut e);
        keys(&mut e, "0");
        e.vim_set_clipboard(false, Some("XY".into()));
        keys(&mut e, "\"*p");
        assert_eq!(e.buffer, "aXYb");
        e.vim_set_clipboard(false, Some("line\n".into()));
        keys(&mut e, "\"+P");
        assert_eq!(e.buffer, "lineaXYb");
        // linewise text reaches the clipboard with its newline
        let mut e = ed("echo hi");
        esc(&mut e);
        keys(&mut e, "\"+yy");
        assert_eq!(e.vim_take_clipboard().as_deref(), Some("echo hi\n"));
    }

    /// The app reads the system clipboard only for a key that may put it:
    /// never for a put from the unnamed register under the default setting.
    #[test]
    fn clipboard_read_only_for_clipboard_puts() {
        let p = VimKey::Char('p');
        let dot = VimKey::Char('.');
        let mut e = ed("ab");
        esc(&mut e);
        assert!(!e.vim_key_reads_clipboard(p, false));
        assert!(e.vim_key_reads_clipboard(p, true));
        keys(&mut e, "\"+");
        assert!(e.vim_key_reads_clipboard(p, false));
        e.vim_set_clipboard(false, Some("Z".into()));
        keys(&mut e, "p");
        assert!(e.vim_key_reads_clipboard(dot, false), "repeats a \"+p");
        keys(&mut e, "\"a");
        assert!(!e.vim_key_reads_clipboard(p, true), "a named register");
        esc(&mut e);
        keys(&mut e, "xp");
        assert!(!e.vim_key_reads_clipboard(dot, false));
        assert!(e.vim_key_reads_clipboard(dot, true));
        assert!(!e.vim_key_reads_clipboard(VimKey::Char('x'), true));
        keys(&mut e, "i");
        assert!(!e.vim_key_reads_clipboard(p, true), "insert mode");
    }

    #[test]
    fn vim_clipboard_unnamed_setting() {
        // unnamed: every yank/delete goes to the clipboard, p reads it
        let mut e = ed("abc");
        esc(&mut e);
        e.vim_set_clipboard(true, None);
        keys(&mut e, "0x");
        assert_eq!(e.vim_take_clipboard().as_deref(), Some("a"));
        e.vim_set_clipboard(true, Some("Z".into()));
        keys(&mut e, "p");
        assert_eq!(e.buffer, "bZc");
        // a named register still bypasses the clipboard
        e.vim_set_clipboard(true, None);
        keys(&mut e, "\"ayl");
        assert_eq!(e.vim_take_clipboard(), None);
        // none (default): the clipboard is left alone
        let mut e = ed("abc");
        esc(&mut e);
        e.vim_set_clipboard(false, Some("Z".into()));
        keys(&mut e, "0xp");
        assert_eq!(e.vim_take_clipboard(), None);
        assert_eq!(e.buffer, "bac");
    }

    #[test]
    fn star_and_hash_search_the_word_under_the_cursor() {
        let s = "cat concat cat dog cat";
        // * from the first "cat" skips "concat" (whole words only)
        let e = run(s, 0, "*");
        assert_eq!(e.cursor, 11);
        let e = run(s, 0, "**");
        assert_eq!(e.cursor, 19);
        // n repeats it whole-word too, wrapping
        let e = run(s, 0, "*nn");
        assert_eq!(e.cursor, 0);
        // # goes back, from inside the word
        let e = run(s, 20, "#");
        assert_eq!(e.cursor, 11);
        // on a blank: the next word on the line
        let e = run(s, 14, "*");
        assert_eq!(e.cursor, 15);
        // d* deletes up to the next match
        let (buf, _, _) = after("ab x ab", 0, "d*");
        assert_eq!(buf, "ab");
    }

    #[test]
    fn line_text_objects() {
        let (buf, _, reg) = after("  echo hi  ", 4, "dil");
        assert_eq!(buf, "    ");
        assert_eq!(reg, "echo hi");
        let (buf, _, reg) = after("  echo hi  ", 4, "dal");
        assert_eq!(buf, "");
        assert_eq!(reg, "  echo hi  ");
        // the cursor's line only
        let (buf, _, _) = after("ab\n cd \nef", 4, "cil");
        assert_eq!(buf, "ab\n  \nef");
        assert_eq!(sel("ab\n cd \nef", 4, "vil").as_deref(), Some("cd"));
    }

    #[test]
    fn history_keys_report_outcomes() {
        let mut e = ed("first");
        e.submit();
        e.insert("second");
        e.submit();
        assert_eq!(e.buffer, "");
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        esc(&mut e);

        assert_eq!(e.vim_key(VimKey::Char('k')), VimOutcome::HistoryUp);
        assert_eq!(e.vim_key(VimKey::Char('j')), VimOutcome::HistoryDown);
        assert_eq!(e.vim_key(VimKey::Up), VimOutcome::HistoryUp);
        assert_eq!(e.vim_key(VimKey::Down), VimOutcome::HistoryDown);
        // the outcome does not mutate; the app runs the recall, and the next
        // normal-mode key re-clamps the cursor onto a char
        assert_eq!(e.buffer, "");
        e.history_up();
        assert_eq!(e.buffer, "second");
        assert_eq!(e.cursor, 6);
        keys(&mut e, "h");
        assert_eq!(e.cursor, 4);
    }

    #[test]
    fn enter_submits_and_submit_resets_to_insert() {
        let mut e = ed("ls -la");
        esc(&mut e);
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));
        assert_eq!(e.vim_key(VimKey::Enter), VimOutcome::Submit);
        assert_eq!(e.buffer, "ls -la");

        let cmd = e.submit();
        assert_eq!(cmd, "ls -la");
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));

        // clear() resets the mode too
        e.insert("oops");
        esc(&mut e);
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));
        e.clear();
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        assert_eq!(e.buffer, "");
    }

    //              0123456789012345
    const HW: &str = "echo hello world";

    #[test]
    fn visual_v_selects_inclusively_and_motions_extend() {
        let e = run(HW, 5, "v");
        assert_eq!(e.vim_mode(), Some(VimMode::Visual));
        assert_eq!(e.selected_text(), Some("h"));
        assert_eq!(sel(HW, 5, "ve").as_deref(), Some("hello"));
        assert_eq!(sel(HW, 5, "vew").as_deref(), Some("hello w"));
        assert_eq!(sel(HW, 5, "v$").as_deref(), Some("hello world"));
        assert_eq!(sel(HW, 5, "vh").as_deref(), Some(" h"));
        assert_eq!(sel(HW, 5, "vfo").as_deref(), Some("hello"));
        assert_eq!(sel(HW, 6, "viw").as_deref(), Some("hello"));
        // Esc leaves with the buffer untouched; `v` again toggles it off
        let mut e = run(HW, 5, "ve");
        esc(&mut e);
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));
        assert_eq!(e.selection(), None);
        assert_eq!(e.cursor, 9);
        assert_eq!(e.buffer, HW);
        assert_eq!(run(HW, 5, "vev").vim_mode(), Some(VimMode::Normal));
        // a selection cleared under it (a click) reads as Normal
        let mut e = run(HW, 5, "ve");
        e.anchor = None;
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));
        keys(&mut e, "x");
        assert_eq!(e.buffer, "echo hell world");
    }

    #[test]
    fn visual_line_selects_whole_lines() {
        //                 0 1 2  3456789 10  11
        const L: &str = "ls\necho hi\npwd";
        let e = run(L, 4, "V");
        assert_eq!(e.vim_mode(), Some(VimMode::VisualLine));
        assert_eq!(e.selected_text(), Some("echo hi"));
        assert_eq!(sel(L, 4, "Vj").as_deref(), Some("echo hi\npwd"));
        assert_eq!(sel(L, 4, "Vk").as_deref(), Some("ls\necho hi"));
        assert_eq!(run(L, 4, "vV").vim_mode(), Some(VimMode::VisualLine));
        assert_eq!(run(L, 4, "Vv").vim_mode(), Some(VimMode::Visual));
        // a linewise delete takes the lines and one newline
        let e = run(L, 4, "Vd");
        assert_eq!(e.buffer, "ls\npwd");
        assert_eq!(e.cursor, 3);
        assert_eq!(e.vim.as_ref().unwrap().register, "echo hi");
        assert!(e.vim.as_ref().unwrap().register_linewise);
        assert_eq!(after(L, 4, "VGd").0, "ls");
        assert_eq!(after(L, 0, "VGd").0, "");
    }

    #[test]
    fn visual_delete() {
        assert_eq!(
            after(HW, 5, "ved"),
            ("echo  world".into(), Some(VimMode::Normal), "hello".into())
        );
        assert_eq!(after(HW, 5, "vex").0, "echo  world");
        // D / X are linewise in visual mode
        assert_eq!(after(HW, 5, "vD").0, "");
    }

    #[test]
    fn visual_change() {
        let e = run(HW, 5, "vec");
        assert_eq!(e.buffer, "echo  world");
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        assert_eq!(e.cursor, 5);
        assert_eq!(e.vim.as_ref().unwrap().register, "hello");
    }

    #[test]
    fn visual_yank() {
        let e = run(HW, 5, "vey");
        assert_eq!(e.buffer, HW);
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));
        assert_eq!(e.selection(), None);
        assert_eq!(e.cursor, 5);
        assert_eq!(e.vim.as_ref().unwrap().register, "hello");
    }

    #[test]
    fn visual_case_operators() {
        assert_eq!(after(HW, 5, "ve~").0, "echo HELLO world");
        assert_eq!(after(HW, 5, "veU").0, "echo HELLO world");
        assert_eq!(after("ECHO", 0, "vlu").0, "ecHO");
        assert_eq!(after(HW, 5, "vegU").0, "echo HELLO world");
        // case operators leave the register alone
        assert_eq!(after(HW, 5, "veU").2, "");
    }

    #[test]
    fn visual_o_swaps_ends() {
        let e = run(HW, 5, "veo");
        assert_eq!(e.cursor, 5);
        assert_eq!(e.selected_text(), Some("hello"));
        assert_eq!(sel(HW, 5, "veoh").as_deref(), Some(" hello"));
    }

    #[test]
    fn indent_and_outdent() {
        // visual: every line of the selection; empty lines stay empty
        assert_eq!(after("a\nb", 0, "Vj>").0, "  a\n  b");
        assert_eq!(after("a\n\nb", 0, "VG>").0, "  a\n\n  b");
        // `<` drops one tab, else up to two spaces
        assert_eq!(after("  a\n\tb\n c\nd", 0, "VG<").0, "a\nb\nc\nd");
        // charwise visual still shifts whole lines
        assert_eq!(after("echo", 1, "vl>").0, "  echo");
        // Normal: >> / << with a count, and > with a motion
        assert_eq!(after("echo", 0, ">>").0, "  echo");
        assert_eq!(after("    x", 0, "<<").0, "  x");
        assert_eq!(after("a\nb\nc", 0, "2>>").0, "  a\n  b\nc");
        assert_eq!(after("a\nb\nc", 0, ">G").0, "  a\n  b\n  c");
    }

    #[test]
    fn visual_join() {
        assert_eq!(after("a\nb\nc", 0, "VGJ").0, "a b c");
    }

    #[test]
    fn dot_repeats_operator_with_motion() {
        assert_eq!(after("one two three four", 0, "dw.").0, "three four");
        assert_eq!(after("one two three four", 0, "d2w.").0, "");
        // a yank in between is not a change
        assert_eq!(after("one two three", 0, "dwyw.").0, "three");
        // nothing to repeat yet
        assert_eq!(after("abc", 0, ".").0, "abc");
    }

    #[test]
    fn dot_repeats_x_r_tilde_p() {
        assert_eq!(after("abcd", 0, "x.").0, "cd");
        // a count replaces the change's count
        assert_eq!(after("abcdef", 0, "x3.").0, "ef");
        assert_eq!(after("abc", 0, "rzl.").0, "zzc");
        assert_eq!(after("abc", 0, "~.").0, "ABc");
        assert_eq!(after("ab", 0, "xp.").0, "baa");
    }

    #[test]
    fn dot_repeats_insert_sessions() {
        let mut e = ed("one two three");
        esc(&mut e);
        keys(&mut e, "gg0cw");
        e.insert("1");
        esc(&mut e);
        assert_eq!(e.buffer, "1 two three");
        keys(&mut e, "w.");
        assert_eq!(e.buffer, "1 1 three");
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));

        let mut e = ed("x");
        esc(&mut e);
        keys(&mut e, "A");
        e.insert("!");
        esc(&mut e);
        keys(&mut e, ".");
        assert_eq!(e.buffer, "x!!");

        let mut e = ed("a");
        esc(&mut e);
        keys(&mut e, "o");
        e.insert("b");
        esc(&mut e);
        keys(&mut e, ".");
        assert_eq!(e.buffer, "a\nb\nb");
    }

    #[test]
    fn dot_repeats_visual_and_replace_changes() {
        assert_eq!(after("abcdefgh", 0, "vld.").0, "efgh");
        let mut e = run("abcdef", 0, "Rxy");
        esc(&mut e);
        assert_eq!(e.buffer, "xycdef");
        keys(&mut e, "l.");
        assert_eq!(e.buffer, "xyxyef");
    }

    #[test]
    fn one_undo_per_change() {
        // a dot repeat
        let mut e = run("one two three", 0, "dw.");
        assert_eq!(e.buffer, "three");
        keys(&mut e, "u");
        assert_eq!(e.buffer, "two three");
        keys(&mut e, "u");
        assert_eq!(e.buffer, "one two three");
        // an Insert session typed in pieces
        let mut e = run("one two", 0, "cw");
        e.insert("a");
        e.insert("b");
        esc(&mut e);
        assert_eq!(e.buffer, "ab two");
        keys(&mut e, "u");
        assert_eq!(e.buffer, "one two");
        // a visual operator, a Replace session, a multi-line join
        let mut e = run(HW, 5, "ved");
        keys(&mut e, "u");
        assert_eq!(e.buffer, HW);
        let mut e = run("abcd", 0, "Rxyz");
        esc(&mut e);
        keys(&mut e, "u");
        assert_eq!(e.buffer, "abcd");
        let mut e = run("a\nb\nc", 0, "3J");
        assert_eq!(e.buffer, "a b c");
        keys(&mut e, "u");
        assert_eq!(e.buffer, "a\nb\nc");
        // undo puts the cursor where the change began, not where the
        // replayed motion moved it
        let mut e = run("ab cd ef", 3, "vld0.");
        assert_eq!(e.buffer, "  ef");
        keys(&mut e, "u");
        assert_eq!((e.buffer.as_str(), e.cursor), ("ab  ef", 0));
    }

    //                 0123456789012345678901234
    const GM: &str = "git commit -m msg --amend";

    #[test]
    fn search_forward_backward_and_repeat() {
        let mut e = run(GM, 0, "/m");
        assert_eq!(e.vim_prompt().as_deref(), Some("/m"));
        assert_eq!(e.vim_mode(), Some(VimMode::Normal));
        assert_eq!(e.vim_key(VimKey::Enter), VimOutcome::Consumed);
        assert_eq!(e.vim_prompt(), None);
        assert_eq!(e.cursor, 6);
        keys(&mut e, "n");
        assert_eq!(e.cursor, 7);
        keys(&mut e, "N");
        assert_eq!(e.cursor, 6);
        // ? searches backward, wrapping; n keeps its direction
        keys(&mut e, "?m");
        e.vim_key(VimKey::Enter);
        assert_eq!(e.cursor, 21);
        keys(&mut e, "n");
        assert_eq!(e.cursor, 14);
        keys(&mut e, "N");
        assert_eq!(e.cursor, 21);
        // forward wraps; an empty pattern reuses the last one
        keys(&mut e, "/");
        e.vim_key(VimKey::Enter);
        assert_eq!(e.cursor, 6);
        // Esc and Backspace on an empty prompt cancel; no match stays put
        keys(&mut e, "/zz");
        esc(&mut e);
        assert_eq!((e.vim_prompt(), e.cursor), (None, 6));
        keys(&mut e, "/q");
        e.vim_key(VimKey::Backspace);
        assert_eq!(e.vim_prompt().as_deref(), Some("/"));
        e.vim_key(VimKey::Backspace);
        assert_eq!(e.vim_prompt(), None);
        keys(&mut e, "/zz");
        e.vim_key(VimKey::Enter);
        assert_eq!(e.cursor, 6);
        // as an operator target (exclusive)
        let mut e = run(GM, 0, "d/co");
        e.vim_key(VimKey::Enter);
        assert_eq!(e.buffer, "commit -m msg --amend");
    }

    #[test]
    fn percent_matches_brackets() {
        //               01234567
        const B: &str = "f(a[b]c)";
        assert_eq!(run(B, 0, "%").cursor, 7);
        assert_eq!(run(B, 0, "%%").cursor, 1);
        assert_eq!(run(B, 4, "%").cursor, 3);
        assert_eq!(after(B, 1, "d%").0, "f");
        assert_eq!(run("(a\nb)", 0, "%").cursor, 4);
        // nested pairs of the same kind
        assert_eq!(run("((a) b)", 0, "%").cursor, 6);
        assert_eq!(run("((a) b)", 6, "%").cursor, 0);
        assert_eq!(run("none", 0, "%").cursor, 0);
    }

    #[test]
    fn comma_reverses_find() {
        //                0123456789
        let mut e = run("a bad cab", 0, "fa");
        assert_eq!(e.cursor, 3);
        keys(&mut e, ";");
        assert_eq!(e.cursor, 7);
        keys(&mut e, ",");
        assert_eq!(e.cursor, 3);
        keys(&mut e, ",");
        assert_eq!(e.cursor, 0);
        keys(&mut e, ",");
        assert_eq!(e.cursor, 0);
    }

    //                0123 456789 01234
    const ML: &str = "one\n  two\nthree";

    #[test]
    fn gg_and_capital_g_go_to_lines() {
        assert_eq!(run(ML, 0, "G").cursor, 10);
        assert_eq!(run(ML, 0, "Ggg").cursor, 0);
        assert_eq!(run(ML, 0, "2G").cursor, 6);
        assert_eq!(run(ML, 12, "2gg").cursor, 6);
        assert_eq!(run(ML, 0, "9G").cursor, 10);
        // linewise operator targets
        assert_eq!(after(ML, 0, "dG").0, "");
        assert_eq!(after(ML, 4, "dG").0, "one");
        assert_eq!(after(ML, 12, "dgg").0, "");
        assert_eq!(after(ML, 4, "yG").2, "  two\nthree");
    }

    #[test]
    fn o_and_capital_o_open_lines() {
        let mut e = run("ls\npwd", 0, "o");
        assert_eq!((e.buffer.as_str(), e.cursor), ("ls\n\npwd", 3));
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        e.insert("cd");
        assert_eq!(e.buffer, "ls\ncd\npwd");
        let mut e = run("ls", 1, "O");
        assert_eq!((e.buffer.as_str(), e.cursor), ("\nls", 0));
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        e.insert("cd");
        assert_eq!(e.buffer, "cd\nls");
    }

    #[test]
    fn capital_j_joins_lines() {
        let e = run("echo\n  hi", 0, "J");
        assert_eq!((e.buffer.as_str(), e.cursor), ("echo hi", 4));
        assert_eq!(after("a(\n)", 0, "J").0, "a()");
        assert_eq!(after("a\n\nb", 0, "J").0, "a\nb");
        assert_eq!(after("a\nb\nc", 0, "3J").0, "a b c");
        assert_eq!(after("last", 0, "J").0, "last");
    }

    #[test]
    fn capital_r_replace_mode() {
        let mut e = run("abcd", 1, "R");
        assert_eq!(e.vim_mode(), Some(VimMode::Replace));
        keys(&mut e, "xy");
        assert_eq!((e.buffer.as_str(), e.cursor), ("axyd", 3));
        esc(&mut e);
        assert_eq!((e.vim_mode(), e.cursor), (Some(VimMode::Normal), 2));
        // past the end it appends
        assert_eq!(run("ab", 1, "Rxyz").buffer, "axyz");
        // Backspace puts back what was overwritten, and drops what was appended
        let mut e = run("ab", 1, "Rxy");
        assert_eq!(e.buffer, "axy");
        e.vim_key(VimKey::Backspace);
        assert_eq!(e.buffer, "ax");
        e.vim_key(VimKey::Backspace);
        assert_eq!((e.buffer.as_str(), e.cursor), ("ab", 1));
    }

    #[test]
    fn ge_moves_to_previous_word_end() {
        //                0123456789012345
        const S: &str = "foo bar-baz qux";
        let mut e = run(S, 12, "ge");
        assert_eq!(e.cursor, 10);
        for want in [7, 6, 2] {
            keys(&mut e, "ge");
            assert_eq!(e.cursor, want, "ge landed wrong");
        }
        let mut e = run(S, 12, "gE");
        assert_eq!(e.cursor, 10);
        keys(&mut e, "gE");
        assert_eq!(e.cursor, 2);
        // inclusive as an operator target
        assert_eq!(after(S, 12, "dge").0, "foo bar-baux");
    }

    #[test]
    fn underscore_is_first_non_blank_and_linewise() {
        assert_eq!(run("  echo", 5, "_").cursor, 2);
        assert_eq!(run("a\n  b", 0, "2_").cursor, 4);
        assert_eq!(after("a\nb", 0, "d_").0, "b");
        assert_eq!(
            after("a\nb", 2, "c_"),
            ("a\n".into(), Some(VimMode::Insert), "b".into())
        );
    }

    #[test]
    fn g_case_operators() {
        assert_eq!(after("Echo Hi", 0, "guiw").0, "echo Hi");
        assert_eq!(after("echo hi", 0, "gUw").0, "ECHO hi");
        assert_eq!(after("abc", 0, "gU$").0, "ABC");
        assert_eq!(after("echo hi", 0, "gUU").0, "ECHO HI");
        assert_eq!(after("ECHO HI", 0, "gugu").0, "echo hi");
        assert_eq!(after("Echo", 0, "g~~").0, "eCHO");
        assert_eq!(after("ab cd", 0, "g~w").0, "AB cd");
        // they leave the register alone and stay in Normal
        assert_eq!(
            after("abc", 0, "gUU"),
            ("ABC".into(), Some(VimMode::Normal), "".into())
        );
    }

    #[test]
    fn line_motions_stay_on_the_cursor_line() {
        let mut e = run(ML, 5, "$");
        assert_eq!(e.cursor, 8);
        keys(&mut e, "0");
        assert_eq!(e.cursor, 4);
        keys(&mut e, "^");
        assert_eq!(e.cursor, 6);
        assert_eq!(after(ML, 6, "D").0, "one\n  \nthree");
        assert_eq!(after(ML, 6, "d$").0, "one\n  \nthree");
        let e = run(ML, 5, "A");
        assert_eq!(e.cursor, 9);
    }
}
