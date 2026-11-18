//! Scrollback kept across an update restart (durable sessions).
//!
//! A window that reattaches a shell rebuilds its pane from the output the
//! session service retained, at most [`crate::daemon::MAX_RETAINED`] bytes.
//! A program that redraws all the time (an agent's spinner) fills that ring
//! with redraws, so its pane would come back with only its last screens.
//! Before an update restart each durable pane writes its scrollback here
//! ([`Kept`]): the lines as styled text, and marks tying output positions
//! to the lines then drawn above the cursor's. The new window learns where
//! the service's replay starts, feeds the lines output before that point
//! had drawn ([`Kept::prelude`]), then the replay, which starts on an empty
//! screen: no line twice, none lost. (A program that redraws lines above
//! the cursor, like an agent's prompt box, may show its last frame once
//! more where the two meet.)

use std::collections::VecDeque;
use std::path::PathBuf;

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use serde::{Deserialize, Serialize};

/// How far back marks reach: twice what the service retains, so a replay
/// starting anywhere in its ring finds one at or before its start.
const MARK_SPAN: u64 = 2 * crate::daemon::MAX_RETAINED as u64;
/// At most this many marks (16 bytes each); past it, every other one of
/// the older half goes (the oldest output is the least likely to be the
/// replay's start, and a mark further back loses a few lines, never more).
const MAX_MARKS: usize = 1 << 15;
/// A kept file this old is from a restart that never finished.
const STALE: std::time::Duration = std::time::Duration::from_secs(3600);
/// A kept file larger than this is not read (a full 100,000-line history
/// of styled 200-column lines stays well under it).
const MAX_FILE: u64 = 256 << 20;

/// Output positions and the lines they had drawn: `(seq, boundary)` says
/// that once the service's output up to byte `seq` was parsed, every line
/// above abs line `boundary` (the cursor's) was drawn.
#[derive(Debug, Default, Clone)]
pub(crate) struct Marks(VecDeque<(u64, u64)>, Option<u64>);

impl Marks {
    /// Output up to byte `seq` was parsed and the terminal shows the
    /// alternate screen (`alt`) or not: remembers the first such `seq` of
    /// the stretch it has been on it without a break.
    pub(crate) fn note_alt(&mut self, seq: u64, alt: bool) {
        if !alt {
            self.1 = None;
        } else if self.1.is_none() {
            self.1 = Some(seq);
        }
    }

    /// The output position from which the terminal has been on the
    /// alternate screen until now, if it is.
    pub(crate) fn alt_since(&self) -> Option<u64> {
        self.1
    }

    /// Output up to byte `seq` was parsed; lines above `boundary` are
    /// drawn.
    pub(crate) fn note(&mut self, seq: u64, boundary: u64) {
        if self.0.back().is_some_and(|last| last.1 == boundary) {
            return;
        }
        self.0.push_back((seq, boundary));
        let floor = seq.saturating_sub(MARK_SPAN);
        while self.0.len() > 1 && self.0[1].0 <= floor {
            self.0.pop_front();
        }
        if self.0.len() > MAX_MARKS {
            let half = self.0.len() / 2;
            let mut i = 0;
            self.0.retain(|_| {
                i += 1;
                i > half || i % 2 == 1
            });
        }
    }

    /// Carry the boundaries across a reflow (`map`: an abs line's new one).
    pub(crate) fn remap(&mut self, map: impl Fn(u64) -> u64) {
        for m in &mut self.0 {
            m.1 = map(m.1);
        }
    }

    pub(crate) fn list(&self) -> Vec<(u64, u64)> {
        self.0.iter().copied().collect()
    }
}

/// A pane's scrollback, kept for the window that reattaches its shell.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    /// `(seq, boundary)` marks: once the output up to byte `seq` was parsed,
    /// every line above abs line `boundary` was drawn. Oldest first.
    pub marks: Vec<(u64, u64)>,
    /// The grid's logical (unwrapped) lines, oldest first: the abs line of
    /// each one's first row and its text with SGR styles.
    pub lines: Vec<(u64, String)>,
    /// The terminal's modes then, as the sequences that set them
    /// (`modes_of`): alternate screen, mouse reporting, bracketed paste,
    /// keyboard protocol, cursor keys.
    #[serde(default)]
    pub modes: String,
    /// The output position from which the terminal had been on the
    /// alternate screen without a break, if it was on it.
    #[serde(default)]
    pub alt_since: Option<u64>,
}

impl Kept {
    /// What to parse before a replay starting at byte `from`: the lines
    /// output before `from` had drawn above the cursor's, each ending in
    /// CRLF. None when the replay draws them all (it starts at the shell's
    /// first byte, or before every mark).
    pub fn prelude(&self, from: u64) -> Option<Vec<u8>> {
        let (_, boundary) = self.marks.iter().rev().find(|m| m.0 <= from)?;
        let mut out = Vec::new();
        for (_, text) in self.lines.iter().take_while(|l| l.0 < *boundary) {
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(b"\x1b[0m\r\n");
        }
        (!out.is_empty()).then_some(out)
    }

    /// With [`Self::modes_before`]: the shell's screen under the full-screen
    /// program, the lines [`Self::prelude`] leaves out (it stops at the
    /// history: a replay from inside the program's run never draws the
    /// screen the program covers). Each line ends in CRLF, so the cursor is
    /// below them, where the shell left it.
    pub fn screen_before(&self, from: u64) -> Vec<u8> {
        let boundary = self
            .marks
            .iter()
            .rev()
            .find(|m| m.0 <= from)
            .map_or(0, |m| m.1);
        let rest: Vec<&(u64, String)> = self.lines.iter().filter(|l| l.0 >= boundary).collect();
        let used = rest
            .iter()
            .rposition(|l| !l.1.is_empty())
            .map_or(0, |i| i + 1);
        let mut out = Vec::new();
        for (_, text) in &rest[..used] {
            out.extend_from_slice(text.as_bytes());
            out.extend_from_slice(b"\x1b[0m\r\n");
        }
        out
    }

    /// The mode sequences to parse before a replay starting at byte `from`:
    /// a full-screen program that has run since before `from` set its modes
    /// in output the service no longer holds (an agent redraws megabytes an
    /// hour), so the replay alone would draw its screens over the shell's
    /// and leave the mouse and the keyboard as a shell has them. None when
    /// the replay holds them itself: it starts at the first byte, or the
    /// program entered the alternate screen after `from`.
    pub fn modes_before(&self, from: u64) -> Option<&[u8]> {
        let since = self.alt_since?;
        (from > 0 && since <= from && !self.modes.is_empty()).then_some(self.modes.as_bytes())
    }
}

/// The sequences that put a fresh terminal into `mode`.
pub(crate) fn modes_of(mode: alacritty_terminal::term::TermMode) -> String {
    use alacritty_terminal::term::TermMode as M;
    let mut out = String::new();
    for (flag, on) in [
        (M::APP_CURSOR, "\x1b[?1h"),
        (M::APP_KEYPAD, "\x1b="),
        (M::MOUSE_REPORT_CLICK, "\x1b[?1000h"),
        (M::MOUSE_DRAG, "\x1b[?1002h"),
        (M::MOUSE_MOTION, "\x1b[?1003h"),
        (M::UTF8_MOUSE, "\x1b[?1005h"),
        (M::SGR_MOUSE, "\x1b[?1006h"),
        (M::FOCUS_IN_OUT, "\x1b[?1004h"),
        (M::BRACKETED_PASTE, "\x1b[?2004h"),
    ] {
        if mode.contains(flag) {
            out.push_str(on);
        }
    }
    if !mode.contains(M::ALTERNATE_SCROLL) {
        out.push_str("\x1b[?1007l");
    }
    let kitty = [
        (M::DISAMBIGUATE_ESC_CODES, 1),
        (M::REPORT_EVENT_TYPES, 2),
        (M::REPORT_ALTERNATE_KEYS, 4),
        (M::REPORT_ALL_KEYS_AS_ESC, 8),
        (M::REPORT_ASSOCIATED_TEXT, 16),
    ]
    .iter()
    .filter(|(flag, _)| mode.contains(*flag))
    .map(|(_, bit)| bit)
    .sum::<u32>();
    if mode.contains(M::ALT_SCREEN) {
        out.push_str("\x1b[?1049h");
    }
    // after the screen switch: each screen has its own keyboard flags
    if kitty > 0 {
        out.push_str(&format!("\x1b[>{kitty}u"));
    }
    if !mode.contains(M::SHOW_CURSOR) {
        out.push_str("\x1b[?25l");
    }
    out
}

/// `grid`'s rows (history and screen) as logical lines, the top row at abs
/// line `scrolled - history`.
pub(crate) fn lines_of(grid: &Grid<Cell>, scrolled: u64) -> Vec<(u64, String)> {
    let history = grid.history_size();
    let rows = history + grid.screen_lines();
    let cols = grid.columns();
    let base = scrolled.saturating_sub(history as u64);
    let mut out = Vec::new();
    let mut line: Option<(u64, String, Pen)> = None;
    for r in 0..rows {
        let row = &grid[Line(r as i32 - history as i32)];
        let wrapped = cols > 0 && row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        let (_, text, pen) =
            line.get_or_insert_with(|| (base + r as u64, String::new(), Pen::default()));
        // a wrapped row is written whole, so the line wraps where it did
        let end = if wrapped {
            cols
        } else {
            (0..cols)
                .rev()
                .find(|&c| !blank(&row[Column(c)]))
                .map_or(0, |c| c + 1)
        };
        for c in 0..end {
            let cell = &row[Column(c)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            pen.write(cell, text);
            text.push(cell.c);
            if let Some(zw) = cell.zerowidth() {
                text.extend(zw);
            }
        }
        if !wrapped && let Some((abs, text, _)) = line.take() {
            out.push((abs, text));
        }
    }
    if let Some((abs, text, _)) = line {
        out.push((abs, text));
    }
    out
}

/// A cell that draws nothing: trailing ones end a line.
fn blank(cell: &Cell) -> bool {
    cell.c == ' '
        && cell.bg == Color::Named(NamedColor::Background)
        && !cell
            .flags
            .intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
        && cell.zerowidth().is_none_or(<[char]>::is_empty)
}

/// The SGR state written so far.
#[derive(Default, PartialEq)]
struct Pen {
    fg: Option<String>,
    bg: Option<String>,
    ul: Option<String>,
    flags: Vec<&'static str>,
}

impl Pen {
    /// Switch to `cell`'s style when it differs.
    fn write(&mut self, cell: &Cell, out: &mut String) {
        const FLAGS: [(Flags, &str); 11] = [
            (Flags::BOLD, "1"),
            (Flags::DIM, "2"),
            (Flags::ITALIC, "3"),
            (Flags::UNDERLINE, "4"),
            (Flags::DOUBLE_UNDERLINE, "4:2"),
            (Flags::UNDERCURL, "4:3"),
            (Flags::DOTTED_UNDERLINE, "4:4"),
            (Flags::DASHED_UNDERLINE, "4:5"),
            (Flags::BLINK, "5"),
            (Flags::INVERSE, "7"),
            (Flags::STRIKEOUT, "9"),
        ];
        let mut flags: Vec<&str> = FLAGS
            .iter()
            .filter(|(f, _)| cell.flags.contains(*f))
            .map(|(_, p)| *p)
            .collect();
        if cell.flags.contains(Flags::HIDDEN) {
            flags.push("8");
        }
        let next = Pen {
            fg: sgr_color(cell.fg, "38"),
            bg: sgr_color(cell.bg, "48"),
            ul: cell.underline_color().and_then(|c| sgr_color(c, "58")),
            flags,
        };
        if next == *self {
            return;
        }
        out.push_str("\x1b[0");
        for p in next
            .flags
            .iter()
            .copied()
            .chain(next.fg.as_deref())
            .chain(next.bg.as_deref())
            .chain(next.ul.as_deref())
        {
            out.push(';');
            out.push_str(p);
        }
        out.push('m');
        *self = next;
    }
}

/// `color` as SGR parameters for `lead` (38 foreground, 48 background, 58
/// underline); None for the default. The 16 named colors stay named (30–37,
/// 90–97 and their backgrounds): bold text brightens only those.
fn sgr_color(color: Color, lead: &str) -> Option<String> {
    let index = match color {
        Color::Spec(rgb) => return Some(format!("{lead};2;{};{};{}", rgb.r, rgb.g, rgb.b)),
        Color::Indexed(i) => i as usize,
        Color::Named(n) => {
            let n = n as usize;
            let dim = NamedColor::DimBlack as usize;
            let n = if (dim..dim + 8).contains(&n) {
                n - dim
            } else {
                n
            };
            let base = match lead {
                "38" => 30,
                "48" => 40,
                _ if n < 16 => return Some(format!("{lead};5;{n}")),
                _ => return None,
            };
            return match n {
                0..8 => Some((base + n).to_string()),
                8..16 => Some((base + 60 + n - 8).to_string()),
                _ => None,
            };
        }
    };
    Some(format!("{lead};5;{index}"))
}

fn dir() -> Option<PathBuf> {
    rift_core::session_state::restart_dir()
}

fn path_in(dir: &std::path::Path, service: u64) -> PathBuf {
    dir.join(format!("scrollback-{service}.json"))
}

/// Write `kept` for the window that reattaches `service` (readable by the
/// user only: scrollback can hold secrets).
pub fn save(service: u64, kept: &Kept) -> std::io::Result<()> {
    let dir = dir().ok_or_else(|| std::io::Error::other("no data directory"))?;
    save_in(&dir, service, kept)
}

fn save_in(dir: &std::path::Path, service: u64, kept: &Kept) -> std::io::Result<()> {
    use rift_core::sys::UnixMode;
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let path = path_in(dir, service);
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .unix_mode(0o600)
        .open(&tmp)?;
    f.write_all(&serde_json::to_vec(kept).map_err(std::io::Error::other)?)?;
    drop(f);
    rift_core::sys::rename(&tmp, &path)
}

/// The scrollback kept for `service`, removed as it is read; None when
/// there is none or it is stale.
pub fn take(service: u64) -> Option<Kept> {
    take_in(&dir()?, service)
}

fn take_in(dir: &std::path::Path, service: u64) -> Option<Kept> {
    let path = path_in(dir, service);
    let fresh = std::fs::metadata(&path).is_ok_and(|m| {
        m.len() <= MAX_FILE
            && m.modified()
                .is_ok_and(|at| at.elapsed().is_ok_and(|age| age < STALE))
    });
    let body = fresh.then(|| std::fs::read(&path).ok()).flatten();
    let _ = std::fs::remove_file(&path);
    serde_json::from_slice(&body?).ok()
}

/// Remove kept scrollback no window took (a restart that failed).
pub fn remove_stale() {
    if let Some(dir) = dir() {
        remove_stale_in(&dir);
    }
}

fn remove_stale_in(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let ours = name.to_str().is_some_and(|n| n.starts_with("scrollback-"));
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|at| at.elapsed().is_ok_and(|age| age >= STALE));
        if ours && old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn marks_keep_one_at_or_before_any_retained_start() {
        let mut m = Marks::default();
        m.note(100, 5);
        m.note(150, 5); // no new line: no mark
        m.note(200, 6);
        m.note(210, 7);
        assert_eq!(m.list(), vec![(100, 5), (200, 6), (210, 7)]);
        m.note(MARK_SPAN + 205, 9);
        // the newest mark at or before seq - MARK_SPAN stays
        assert_eq!(m.list(), vec![(200, 6), (210, 7), (MARK_SPAN + 205, 9)]);
    }

    #[test]
    fn too_many_marks_thin_the_older_half() {
        let mut m = Marks::default();
        for i in 0..=MAX_MARKS as u64 {
            m.note(i, i);
        }
        let list = m.list();
        assert!(list.len() <= MAX_MARKS * 3 / 4 + 1, "{}", list.len());
        // the newest stay one per line, the oldest still reach back
        assert_eq!(list.first(), Some(&(0, 0)));
        let tail: Vec<u64> = list.iter().rev().take(100).map(|m| m.0).collect();
        assert!(tail.windows(2).all(|w| w[0] == w[1] + 1));
    }

    #[test]
    fn a_prelude_holds_what_the_replay_will_not_draw() {
        let kept = Kept {
            marks: vec![(100, 2), (500, 4)],
            lines: (0..6).map(|i| (i, format!("l{i}"))).collect(),
            ..Default::default()
        };
        // the replay starts at the shell's first byte: it draws everything
        assert_eq!(kept.prelude(0), None);
        assert_eq!(kept.prelude(99), None);
        assert_eq!(
            kept.prelude(100).unwrap(),
            b"l0\x1b[0m\r\nl1\x1b[0m\r\n".to_vec()
        );
        let late = String::from_utf8(kept.prelude(10_000).unwrap()).unwrap();
        assert_eq!(late, "l0\x1b[0m\r\nl1\x1b[0m\r\nl2\x1b[0m\r\nl3\x1b[0m\r\n");
    }

    #[test]
    fn kept_files_are_private_taken_once_and_cleared_when_stale() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rift-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let kept = Kept {
            marks: vec![(1, 1)],
            lines: vec![(0, "hi".into())],
            ..Default::default()
        };
        save_in(&dir, 7, &kept).unwrap();
        let mode = std::fs::metadata(path_in(&dir, 7))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(take_in(&dir, 7), Some(kept.clone()));
        assert_eq!(take_in(&dir, 7), None);
        // a file left by a restart that never finished
        save_in(&dir, 8, &kept).unwrap();
        let old = std::time::SystemTime::now() - STALE - std::time::Duration::from_secs(1);
        std::fs::File::options()
            .write(true)
            .open(path_in(&dir, 8))
            .unwrap()
            .set_modified(old)
            .unwrap();
        save_in(&dir, 9, &kept).unwrap();
        remove_stale_in(&dir);
        assert!(!path_in(&dir, 8).exists());
        assert!(path_in(&dir, 9).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn modes_of_names_what_a_full_screen_program_set() {
        use alacritty_terminal::term::TermMode as M;
        // a shell at its prompt: nothing but what a fresh terminal has
        assert_eq!(
            modes_of(M::SHOW_CURSOR | M::LINE_WRAP | M::ALTERNATE_SCROLL),
            ""
        );
        let agent = M::LINE_WRAP
            | M::ALTERNATE_SCROLL
            | M::ALT_SCREEN
            | M::MOUSE_REPORT_CLICK
            | M::SGR_MOUSE
            | M::BRACKETED_PASTE
            | M::DISAMBIGUATE_ESC_CODES
            | M::REPORT_ALTERNATE_KEYS;
        assert_eq!(
            modes_of(agent),
            "\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?1049h\x1b[>5u\x1b[?25l"
        );
        let kept = Kept {
            modes: modes_of(agent),
            alt_since: Some(100),
            ..Default::default()
        };
        // only a replay that starts inside the program's run needs them
        assert!(kept.modes_before(100).is_some());
        assert!(kept.modes_before(5000).is_some());
        assert_eq!(kept.modes_before(99), None);
        assert_eq!(kept.modes_before(0), None);
        assert_eq!(
            Kept {
                alt_since: None,
                ..kept
            }
            .modes_before(5000),
            None
        );
    }
}
