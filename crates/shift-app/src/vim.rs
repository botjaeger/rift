//! Warp-style vim modal editing for the owned command-line editor.
//!
//! The editor buffer is a single line, so every motion here is a within-line
//! motion and the "linewise" operators (`dd`, `cc`/`S`, `yy`) act on the whole
//! buffer. In Normal mode the cursor sits *on* a character — it is the byte
//! offset of that char and is clamped to the last char whenever the buffer is
//! non-empty (0 when it is empty).
//!
//! Every mutation goes through [`EditorState::replace_range`], the single
//! buffer-mutation point that keeps workflow placeholder spans valid and
//! pushes undo snapshots.

use std::ops::Range;

use crate::state::EditorState;

/// Which vim mode the owned editor is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VimMode {
    #[default]
    Insert,
    Normal,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Motion {
    Left,
    Right,
    LineStart,
    LineEnd,
    FirstNonBlank,
    /// `w` / `W` (`true` = whitespace-delimited).
    WordFwd(bool),
    /// `b` / `B`.
    WordBack(bool),
    /// `e` / `E`.
    WordEnd(bool),
    Find(FindKind, char),
}

/// A key that still needs a char argument (`r`, `f`, `F`, `t`, `T`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Awaiting {
    Replace,
    Find(FindKind),
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

    /// Effective count: vim multiplies the two (`2d3w` = six words).
    fn total(&self) -> usize {
        self.count
            .unwrap_or(1)
            .saturating_mul(self.op_count.unwrap_or(1))
            .max(1)
    }
}

/// Modal-editing state hung off [`EditorState`]; `None` there = vim disabled.
#[derive(Debug, Default)]
pub struct VimState {
    pub mode: VimMode,
    pending: Pending,
    /// Unnamed register (`x`, `d`, `c`, `y` fill it; `p`/`P` read it).
    register: String,
    /// The register holds a whole line (`dd`, `cc`, `yy`).
    register_linewise: bool,
    /// Last `f`/`F`/`t`/`T` target, repeated by `;`.
    last_find: Option<(FindKind, char)>,
}

impl VimState {
    /// Back to Insert with no half-typed command — what users expect right
    /// after running a command (matches Warp).
    pub fn reset_insert(&mut self) {
        self.mode = VimMode::Insert;
        self.pending.reset();
    }
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

    fn first_non_blank(&self) -> usize {
        self.chars
            .iter()
            .position(|ch| !ch.is_whitespace())
            .unwrap_or(0)
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

/// Resolve a motion to `(target char index, inclusive)`; `None` = it failed.
fn resolve(l: &Line, ci: usize, m: Motion, count: usize) -> Option<(usize, bool)> {
    let n = l.n();
    Some(match m {
        Motion::Left => (ci.saturating_sub(count), false),
        Motion::Right => ((ci + count).min(n), false),
        Motion::LineStart => (0, false),
        Motion::LineEnd => (l.last(), true),
        Motion::FirstNonBlank => (l.first_non_blank(), false),
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
        Motion::Find(kind, ch) => {
            let t = find_char(l, ci, kind, ch, count)?;
            (
                t,
                matches!(kind, FindKind::ForwardTo | FindKind::ForwardTill),
            )
        }
    })
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

impl EditorState {
    /// Feed one key to the vim layer. Returns [`VimOutcome::PassThrough`] when
    /// vim is disabled, or in Insert mode for everything except Escape.
    pub fn vim_key(&mut self, key: VimKey) -> VimOutcome {
        let Some(mut v) = self.vim.take() else {
            return VimOutcome::PassThrough;
        };
        let out = self.dispatch(&mut v, key);
        self.vim = Some(v);
        out
    }

    fn dispatch(&mut self, v: &mut VimState, key: VimKey) -> VimOutcome {
        if v.mode == VimMode::Insert {
            if key != VimKey::Escape {
                return VimOutcome::PassThrough;
            }
            v.mode = VimMode::Normal;
            v.pending.reset();
            // vim steps left on leaving Insert (clamped at column 0)
            let l = Line::new(&self.buffer);
            let ci = l.index(self.cursor).saturating_sub(1);
            self.cursor = l.byte(ci);
            self.vim_clamp();
            return VimOutcome::Consumed;
        }

        // The buffer may have moved under us (history recall, undo, workflow
        // insert), so re-establish the on-a-char invariant every keystroke.
        self.vim_clamp();

        if let Some(aw) = v.pending.awaiting.take() {
            let VimKey::Char(c) = key else {
                v.pending.reset();
                return VimOutcome::Consumed;
            };
            return self.char_arg(v, aw, c);
        }

        match key {
            VimKey::Escape => {
                v.pending.reset();
                VimOutcome::Consumed
            }
            VimKey::Enter => {
                v.pending.reset();
                VimOutcome::Submit
            }
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

    /// Apply a motion: move the cursor, or feed the pending operator.
    fn motion(&mut self, v: &mut VimState, m: Motion) -> VimOutcome {
        let count = v.pending.total();
        let op = v.pending.op;
        let l = Line::new(&self.buffer);
        let ci = l.index(self.cursor);
        let n = l.n();

        // vim special case: `cw` on a non-blank behaves like `ce`.
        let mut m = m;
        if op == Some(Op::Change)
            && let Motion::WordFwd(big) = m
            && ci < n
            && !l.chars[ci].is_whitespace()
        {
            m = Motion::WordEnd(big);
        }

        let Some((t, inclusive)) = resolve(&l, ci, m, count) else {
            v.pending.reset();
            return VimOutcome::Consumed;
        };

        match op {
            None => {
                self.cursor = if n == 0 { 0 } else { l.byte(t.min(n - 1)) };
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

    /// Run an operator over a byte range: yank / delete / change.
    fn apply_op(&mut self, v: &mut VimState, op: Op, range: Range<usize>, linewise: bool) {
        if !range.is_empty() {
            v.register = self.buffer[range.clone()].to_string();
            v.register_linewise = linewise;
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
        }
    }

    /// The char argument of `r` / `f` / `F` / `t` / `T`.
    fn char_arg(&mut self, v: &mut VimState, aw: Awaiting, c: char) -> VimOutcome {
        match aw {
            Awaiting::Find(kind) => {
                v.last_find = Some((kind, c));
                self.motion(v, Motion::Find(kind, c))
            }
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

        // operators; a doubled operator is the linewise form (dd / cc / yy)
        let as_op = match c {
            'd' => Some(Op::Delete),
            'c' => Some(Op::Change),
            'y' => Some(Op::Yank),
            _ => None,
        };
        if let Some(op) = as_op {
            match v.pending.op {
                None => v.pending.op = Some(op),
                Some(prev) if prev == op => {
                    self.apply_op(v, op, 0..self.buffer.len(), true);
                    v.pending.reset();
                    if v.mode == VimMode::Normal {
                        self.vim_clamp();
                    }
                }
                Some(_) => v.pending.reset(),
            }
            return VimOutcome::Consumed;
        }

        let motion = match c {
            'h' => Some(Motion::Left),
            'l' | ' ' => Some(Motion::Right),
            '0' => Some(Motion::LineStart),
            '$' => Some(Motion::LineEnd),
            '^' => Some(Motion::FirstNonBlank),
            'w' => Some(Motion::WordFwd(false)),
            'W' => Some(Motion::WordFwd(true)),
            'b' => Some(Motion::WordBack(false)),
            'B' => Some(Motion::WordBack(true)),
            'e' => Some(Motion::WordEnd(false)),
            'E' => Some(Motion::WordEnd(true)),
            _ => None,
        };
        if let Some(m) = motion {
            return self.motion(v, m);
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
        if c == ';' {
            return match v.last_find {
                Some((k, ch)) => self.motion(v, Motion::Find(k, ch)),
                None => {
                    v.pending.reset();
                    VimOutcome::Consumed
                }
            };
        }

        // anything else while an operator is pending cancels it
        if v.pending.op.is_some() {
            v.pending.reset();
            return VimOutcome::Consumed;
        }

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
                self.cursor = l.byte(l.first_non_blank());
                v.mode = VimMode::Insert;
            }
            'A' => {
                self.cursor = self.buffer.len();
                v.mode = VimMode::Insert;
            }
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
            'C' => self.apply_op(v, Op::Change, self.cursor..self.buffer.len(), false),
            'D' => self.apply_op(v, Op::Delete, self.cursor..self.buffer.len(), false),
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
            'u' => {
                for _ in 0..count {
                    self.undo();
                }
            }
            'U' => {
                for _ in 0..count {
                    self.redo();
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

    /// `p` / `P`. A linewise register has no second line to land on here, so
    /// it goes at the end (`p`) or the start (`P`) of the single line.
    fn put(&mut self, v: &mut VimState, count: usize, after: bool) {
        if v.register.is_empty() {
            return;
        }
        let text = v.register.repeat(count);
        if v.register_linewise {
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
        keys(&mut e, "U");
        assert_eq!(e.buffer, "commit");
        assert_eq!(e.cursor, 0);
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

    #[test]
    fn placeholder_spans_survive_a_delete_before_them() {
        let mut e = EditorState::default();
        e.vim = Some(VimState::default());
        e.set_with_placeholders("ssh host -p port".into(), vec![(4, 8), (12, 16)]);
        esc(&mut e);
        keys(&mut e, "0dw");
        assert_eq!(e.buffer, "host -p port");
        assert_eq!(e.placeholders, vec![(0, 4), (8, 12)]);
        assert_eq!(&e.buffer[0..4], "host");
        assert_eq!(&e.buffer[8..12], "port");
    }
}
