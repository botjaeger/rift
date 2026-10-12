//! VT420 / xterm extensions to [`Term`] (Rift).
//!
//! Upstream alacritty implements a VT220-ish subset. Rift extends it toward
//! xterm's VT420 emulation, measured with esctest: left/right margins
//! (DECLRMM, DECSLRM), rectangle editing (DECCRA, DECERA, DECSERA, DECFRA,
//! DECCARA, DECRARA, DECSACE), selective erase (DECSCA, DECSED, DECSEL),
//! column insert/delete (DECIC, DECDC), DECBI/DECFI, line size attributes,
//! national character sets, soft reset (DECSTR), conformance levels
//! (DECSCL), 8-bit controls (S8C1T) and the reports that go with them
//! (DECRQCRA, DECRQSS, DECRQM, DECXCPR, DECRQPSR, XTWINOPS, XTVERSION).

use std::cmp;
use std::collections::HashMap;

use log::debug;

use crate::event::{Event, EventListener};
use crate::grid::Dimensions;
use crate::index::{Column, Line};
use crate::term::cell::{Cell, Flags};
use crate::term::{TITLE_STACK_MAX_DEPTH, Term, TermMode};
use crate::vte::ansi::{Color, NamedColor, Rect};

/// State Rift tracks beyond upstream's [`TermMode`].
#[derive(Debug, Clone)]
pub(crate) struct Vt420 {
    /// DECLRMM (`CSI ? 69 h`): left/right margins may be set.
    pub(crate) declrmm: bool,
    /// Left and right margins, inclusive. Always the full width while
    /// DECLRMM is reset.
    pub(crate) left: usize,
    pub(crate) right: usize,
    /// DECSCNM (`CSI ? 5 h`): reverse video screen.
    pub(crate) reverse_screen: bool,
    /// DECSACE: DECCARA/DECRARA act on a rectangle (else a stream).
    pub(crate) rect_extent: bool,
    /// S8C1T: replies use 8-bit C1 controls.
    pub(crate) c1_8bit: bool,
    /// DECSCL conformance level (61 = VT100 … 64 = VT420).
    pub(crate) level: u16,
    /// Reverse wraparound (`CSI ? 45 h`) and xterm's extended form (1045).
    pub(crate) reverse_wrap: bool,
    pub(crate) reverse_wrap_extended: bool,
    /// DECCOLM may change the column count (`CSI ? 40 h`).
    pub(crate) allow_column_switch: bool,
    /// DECCOLM state: 132 columns requested.
    pub(crate) columns_132: bool,
    /// DECNCSM (`CSI ? 95 h`): DECCOLM does not clear the screen.
    pub(crate) no_clear_on_column: bool,
    /// Other DEC private and ANSI modes, set or reset (reported by DECRQM).
    pub(crate) private_modes: HashMap<u16, bool>,
    pub(crate) ansi_modes: HashMap<u16, bool>,
    /// OSC 1 icon name.
    pub(crate) icon: Option<String>,
    /// XTWINOPS 22/23: saved (window title, icon name) pairs.
    pub(crate) name_stack: Vec<(Option<String>, Option<String>)>,
    /// DEC locator: reporting enabled (DECELR), one-shot.
    pub(crate) locator: u16,
    pub(crate) locator_one_shot: bool,
    /// Lines per page (DECSLPP) as last requested.
    pub(crate) page_lines: Option<u16>,
    /// The pointer's cell, as the host last reported it.
    pub(crate) locator_pos: Option<(usize, usize)>,
    /// xterm special colors BD, UL, BL, RV, IT (OSC 4;256+n, OSC 5;n).
    pub(crate) special: [Option<crate::vte::ansi::Rgb>; 5],
    /// SPA/EPA: characters written now are ISO protected.
    pub(crate) iso_protect: bool,
    /// xterm's protection mode is ISO (set by SPA): ED, EL and ECH then
    /// keep ISO-protected cells. DECSCA, DECSTR and RIS end it.
    pub(crate) iso_mode: bool,
    /// A program set a DEC line size (DECDWL/DECDHL) at some point: rows may
    /// carry `attr`.
    pub(crate) line_sizes: bool,
    /// The grapheme cluster text is being written into.
    pub(crate) cluster: super::width::Cluster,
    /// XTSAVE: saved DEC private modes.
    pub(crate) saved_modes: HashMap<u16, bool>,
    /// A character resize the program asked for (XTWINOPS 8, DECCOLM,
    /// DECSLPP): rows, columns. The host applies it to the grid at once, so
    /// the program's next query and output already see the new size, and
    /// then resizes the window to match.
    pub(crate) pending_resize: Option<(usize, usize)>,
    /// DECSACE as sent (0, 1 = stream, 2 = rectangle).
    pub(crate) sace: u16,
    /// DECSASD / DECSSDT values (no status line exists).
    pub(crate) status_display: u16,
    pub(crate) status_type: u16,
}

impl Vt420 {
    pub(crate) fn new(columns: usize) -> Self {
        Self {
            declrmm: false,
            left: 0,
            right: columns.saturating_sub(1),
            reverse_screen: false,
            rect_extent: false,
            c1_8bit: false,
            level: 64,
            reverse_wrap: false,
            reverse_wrap_extended: false,
            allow_column_switch: false,
            columns_132: false,
            no_clear_on_column: false,
            private_modes: HashMap::new(),
            ansi_modes: HashMap::new(),
            icon: None,
            name_stack: Vec::new(),
            locator: 0,
            locator_one_shot: false,
            page_lines: None,
            locator_pos: None,
            special: [None; 5],
            iso_protect: false,
            iso_mode: false,
            line_sizes: false,
            cluster: Default::default(),
            saved_modes: HashMap::new(),
            pending_resize: None,
            sace: 0,
            status_display: 0,
            status_type: 0,
        }
    }
}

/// A resolved rectangle: 0-based, inclusive, on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Area {
    pub(crate) top: usize,
    pub(crate) left: usize,
    pub(crate) bottom: usize,
    pub(crate) right: usize,
}

impl<T> Term<T> {
    /// Left and right margins, inclusive.
    #[inline]
    pub(crate) fn lr(&self) -> (usize, usize) {
        if self.vt.declrmm {
            (self.vt.left, cmp::min(self.vt.right, self.columns() - 1))
        } else {
            (0, self.columns() - 1)
        }
    }

    /// Top and bottom margins, inclusive.
    #[inline]
    pub(crate) fn tb(&self) -> (usize, usize) {
        (self.scroll_region.start.0 as usize, self.scroll_region.end.0 as usize - 1)
    }

    /// Whether the left/right margins narrow the screen.
    #[inline]
    pub(crate) fn has_lr_margins(&self) -> bool {
        let (l, r) = self.lr();
        l > 0 || r < self.columns() - 1
    }

    /// The cursor is inside the left/right margins.
    #[inline]
    pub(crate) fn cursor_in_lr(&self) -> bool {
        let (l, r) = self.lr();
        let c = self.grid.cursor.point.column.0;
        l <= c && c <= r
    }

    /// The cursor is inside the top/bottom margins.
    #[inline]
    pub(crate) fn cursor_in_tb(&self) -> bool {
        let (t, b) = self.tb();
        let l = self.grid.cursor.point.line.0 as usize;
        t <= l && l <= b
    }

    /// The cursor as CPR/DECXCPR report it: 1-based, relative to the
    /// margins under DECOM.
    pub(crate) fn cursor_report(&self) -> (usize, usize) {
        let point = self.grid.cursor.point;
        let (mut row, mut col) = (point.line.0 as usize + 1, point.column.0 + 1);
        if self.mode.contains(TermMode::ORIGIN) {
            row -= self.tb().0;
            col -= self.lr().0;
        }
        (row, col)
    }

    /// Resolve a DEC rectangle: 1-based, 0 = default, relative to the
    /// margins under DECOM, clamped to the screen. `None` when empty.
    pub(crate) fn resolve_rect(&self, rect: Rect) -> Option<Area> {
        let rows = self.screen_lines();
        let cols = self.columns();
        let (row0, row_max, col0, col_max) = if self.mode.contains(TermMode::ORIGIN) {
            let (t, b) = self.tb();
            let (l, r) = self.lr();
            (t, b, l, r)
        } else {
            (0, rows - 1, 0, cols - 1)
        };
        let pick = |v: u16, base: usize, default: usize, max: usize| -> usize {
            if v == 0 { default } else { cmp::min(base + v as usize - 1, max) }
        };
        let top = pick(rect.top, row0, row0, row_max);
        let left = pick(rect.left, col0, col0, col_max);
        let bottom = pick(rect.bottom, row0, row_max, row_max);
        let right = pick(rect.right, col0, col_max, col_max);
        (top <= bottom && left <= right).then_some(Area { top, left, bottom, right })
    }

    /// Move the cells of `area` by `rows` (positive = down) and `cols`
    /// (positive = right) inside it, filling the vacated cells with blanks
    /// in the current background.
    pub(crate) fn shift_area(&mut self, area: Area, rows: isize, cols: isize) {
        let blank: Cell = self.grid.cursor.template.bg.into();
        let height = (area.bottom - area.top + 1) as isize;
        let width = (area.right - area.left + 1) as isize;
        if rows.abs() >= height || cols.abs() >= width {
            for line in area.top..=area.bottom {
                for col in area.left..=area.right {
                    self.grid[Line(line as i32)][Column(col)] = blank.clone();
                }
            }
            self.mark_fully_damaged();
            return;
        }
        // copy in the order that never reads an overwritten cell
        let lines: Vec<usize> = if rows > 0 {
            (area.top..=area.bottom).rev().collect()
        } else {
            (area.top..=area.bottom).collect()
        };
        let columns: Vec<usize> = if cols > 0 {
            (area.left..=area.right).rev().collect()
        } else {
            (area.left..=area.right).collect()
        };
        for &line in &lines {
            for &col in &columns {
                let src_line = line as isize - rows;
                let src_col = col as isize - cols;
                let inside = src_line >= area.top as isize
                    && src_line <= area.bottom as isize
                    && src_col >= area.left as isize
                    && src_col <= area.right as isize;
                let cell = if inside {
                    self.grid[Line(src_line as i32)][Column(src_col as usize)].clone()
                } else {
                    blank.clone()
                };
                self.grid[Line(line as i32)][Column(col)] = cell;
            }
        }
        self.mark_fully_damaged();
    }

    /// Erase `area`; `selective` keeps DECSCA-protected cells.
    pub(crate) fn erase_area(&mut self, area: Area, selective: bool) {
        self.erase_area_keeping(
            area,
            selective,
            if selective { Flags::PROTECTED } else { Flags::empty() },
        );
    }

    /// Erase `area`, skipping cells with any of `keep`; `selective` keeps
    /// the attributes of erased cells.
    pub(crate) fn erase_area_keeping(&mut self, area: Area, selective: bool, keep: Flags) {
        let bg = self.grid.cursor.template.bg;
        for line in area.top..=area.bottom {
            for col in area.left..=area.right {
                let cell = &mut self.grid[Line(line as i32)][Column(col)];
                if cell.flags.intersects(keep) {
                    continue;
                }
                if selective {
                    // selective erase keeps the attributes, clears the text
                    cell.c = ' ';
                    cell.flags.remove(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER);
                    cell.extra = None;
                } else {
                    *cell = bg.into();
                }
            }
        }
        self.mark_fully_damaged();
    }

    /// Any cell on screen carries one of `flags`.
    pub(crate) fn screen_has(&self, flags: Flags) -> bool {
        (0..self.screen_lines()).any(|l| {
            let row = &self.grid[Line(l as i32)];
            (0..self.columns()).any(|c| row[Column(c)].flags.intersects(flags))
        })
    }

    /// Checksum of `area` the way DECRQCRA reports it: the negated 16-bit
    /// sum of the characters (blank cells count as spaces).
    pub(crate) fn area_checksum(&self, area: Area) -> u16 {
        let mut sum: u32 = 0;
        for line in area.top..=area.bottom {
            for col in area.left..=area.right {
                let cell = &self.grid[Line(line as i32)][Column(col)];
                sum = sum.wrapping_add(cell.c as u32);
            }
        }
        (0x10000 - (sum & 0xffff)) as u16
    }

    /// The SGR parameters that reproduce the cursor's current rendition
    /// (DECRQSS `m`).
    pub(crate) fn sgr_report(&self) -> String {
        let template = &self.grid.cursor.template;
        let flags = template.flags;
        let mut out = vec![String::from("0")];
        for (flag, code) in [
            (Flags::BOLD, "1"),
            (Flags::DIM, "2"),
            (Flags::ITALIC, "3"),
            (Flags::UNDERLINE, "4"),
            (Flags::BLINK, "5"),
            (Flags::INVERSE, "7"),
            (Flags::HIDDEN, "8"),
            (Flags::STRIKEOUT, "9"),
            (Flags::DOUBLE_UNDERLINE, "21"),
        ] {
            if flags.contains(flag) {
                out.push(code.into());
            }
        }
        let color = |c: Color, base: u8| -> Option<String> {
            match c {
                Color::Named(n) => {
                    let i = n as usize;
                    if i < 8 {
                        Some(format!("{}", base as usize + i))
                    } else if i < 16 {
                        Some(format!("{}", base as usize + 60 + i - 8))
                    } else {
                        None
                    }
                },
                Color::Indexed(i) => Some(format!("{};5;{i}", base + 8)),
                Color::Spec(rgb) => Some(format!("{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b)),
            }
        };
        if template.fg != Color::Named(NamedColor::Foreground) {
            out.extend(color(template.fg, 30));
        }
        if template.bg != Color::Named(NamedColor::Background) {
            out.extend(color(template.bg, 40));
        }
        out.join(";")
    }
}

impl<T: EventListener> Term<T> {
    /// Send a reply. Replies stay 7-bit even after S8C1T: a UTF-8 stream
    /// cannot carry raw C1 bytes (xterm's UTF-8 mode does the same).
    pub(crate) fn reply(&self, text: String) {
        self.event_proxy.send_event(Event::PtyWrite(text));
    }

    /// Reply to DECRQSS `DCS $ q Pt ST`.
    pub(crate) fn report_status_string(&self, request: &[u8]) {
        let (t, b) = self.tb();
        let (l, r) = self.lr();
        let body = match request {
            b"m" => Some(format!("{}m", self.sgr_report())),
            b"r" => Some(format!("{};{}r", t + 1, b + 1)),
            b"s" => Some(format!("{};{}s", l + 1, r + 1)),
            b"\"q" => Some(format!(
                "{}\"q",
                if self.grid.cursor.template.flags.contains(Flags::PROTECTED) { 1 } else { 0 }
            )),
            b"\"p" => Some(format!("{};{}\"p", self.vt.level, if self.vt.c1_8bit { 0 } else { 1 })),
            b" q" => {
                let style = self.cursor_style();
                let n = match style.shape {
                    crate::vte::ansi::CursorShape::Block => 1,
                    crate::vte::ansi::CursorShape::Underline => 3,
                    crate::vte::ansi::CursorShape::Beam => 5,
                    _ => 1,
                } + u8::from(!style.blinking);
                Some(format!("{n} q"))
            },
            b"t" => Some(format!("{}t", self.screen_lines())),
            b"$|" => Some(format!("{}$|", self.columns())),
            b"*|" => Some(format!("{}*|", self.screen_lines())),
            b"*x" => Some(format!("{}*x", self.vt.sace)),
            b"$}" => Some(format!("{}$}}", self.vt.status_display)),
            b"$~" => Some(format!("{}$~", self.vt.status_type)),
            _ => None,
        };
        let text = match body {
            Some(body) => format!("\x1bP1$r{body}\x1b\\"),
            None => String::from("\x1bP0$r\x1b\\"),
        };
        self.reply(text);
    }

    /// Reply to XTGETTCAP `DCS + q hex;… ST`.
    pub(crate) fn report_termcap(&self, request: &[u8]) {
        let hex = |s: &str| s.bytes().map(|b| format!("{b:02X}")).collect::<String>();
        let names: Vec<String> = request
            .split(|&b| b == b';')
            .map(|h| {
                (0..h.len() / 2)
                    .filter_map(|i| std::str::from_utf8(&h[i * 2..i * 2 + 2]).ok())
                    .filter_map(|p| u8::from_str_radix(p, 16).ok())
                    .map(char::from)
                    .collect()
            })
            .collect();
        for name in names {
            let value = match name.as_str() {
                "TN" | "name" => Some(String::from("xterm-256color")),
                "Co" | "colors" => Some(String::from("256")),
                "RGB" => Some(String::from("8/8/8")),
                // styled (curly, dotted, dashed) and colored underlines, in
                // kitty's terminfo spelling: Rift draws both
                "Smulx" => Some(String::from("\x1b[4:%p1%dm")),
                "Setulc" => Some(String::from(
                    "\x1b[58:2::%p1%{65536}%/%d:%p1%{256}%/%{255}%&%d:%p1%{255}%&%dm",
                )),
                _ => None,
            };
            let text = match value {
                Some(v) => format!("\x1bP1+r{}={}\x1b\\", hex(&name), hex(&v)),
                None => format!("\x1bP0+r{}\x1b\\", hex(&name)),
            };
            self.reply(text);
        }
    }

    /// DECRQM reply.
    pub(crate) fn report_mode_state(&self, private: bool, mode: u16, state: u8) {
        let q = if private { "?" } else { "" };
        self.reply(format!("\x1b[{q}{mode};{state}$y"));
    }

    /// Log an unexpected request instead of answering it.
    pub(crate) fn ignore(&self, what: &str) {
        debug!("vt420: ignoring {what}");
    }
}

/// DEC private modes Rift tracks without upstream behavior, with their
/// power-on value: reported by DECRQM, toggled by DECSET/DECRST.
const TRACKED_PRIVATE: &[(u16, bool)] = &[
    (2, true),     // DECANM: ANSI (VT52 mode is not offered)
    (4, false),    // DECSCLM smooth scroll
    (9, false),    // X10 mouse
    (10, false),   // rxvt toolbar
    (18, false),   // DECPFF print form feed
    (19, true),    // DECPEX print extent: full screen
    (38, false),   // DECTEK
    (41, false),   // more(1) fix
    (42, false),   // DECNRCM national replacement
    (44, false),   // margin bell
    (46, false),   // logging
    (66, false),   // DECNKM application keypad
    (67, false),   // DECBKM backarrow sends BS
    (1001, false), // highlight mouse tracking
    (1010, false), // scroll on output
    (1011, true),  // scroll on key press
    (1034, true),  // meta sends 8 bits (UTF-8 input)
    (1035, true),  // special modifiers
    (1036, true),  // meta sends ESC
    (1037, false), // delete sends DEL
    (1039, true),  // alt sends ESC
    (1040, false), // keep selection
    (1041, false), // use CLIPBOARD
    (1043, false), // pop on bell
    (1044, false), // keep CLIPBOARD
    (1050, false), // terminfo function keys
    (1051, false), // Sun function keys
    (1052, false), // HP function keys
    (1053, false), // SCO function keys
    (1060, false), // legacy keyboard
    (1061, false), // VT220 keyboard
    (2001, false), // readline mouse button-1
    (2002, false), // readline mouse button-2
    (2003, false), // readline mouse button-3
    (2005, false), // readline character quoting
    (2006, false), // readline newline pasting
];

/// ANSI modes Rift tracks: KAM (2) and SRM (12, set = no local echo).
const TRACKED_ANSI: &[(u16, bool)] = &[(2, false), (12, true)];

impl<T: EventListener> Term<T> {
    /// DECSET/DECRST of a mode upstream does not name.
    pub(crate) fn set_extended_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            5 => {
                self.vt.reverse_screen = on;
                self.mark_fully_damaged();
            },
            40 => self.vt.allow_column_switch = on,
            45 => self.vt.reverse_wrap = on,
            1045 => self.vt.reverse_wrap_extended = on,
            69 => {
                self.vt.declrmm = on;
                if !on {
                    self.vt.left = 0;
                    self.vt.right = self.columns() - 1;
                }
            },
            95 => self.vt.no_clear_on_column = on,
            // the alternate screen without (47) and with (1047) clearing,
            // and the cursor save on its own (1048)
            47 | 1047 => self.alt_screen(on, mode == 1047),
            1048 => {
                use crate::vte::ansi::Handler;
                if on {
                    self.save_cursor_position();
                } else {
                    self.restore_cursor_position();
                }
            },
            66 => {
                self.mode.set(TermMode::APP_KEYPAD, on);
                self.vt.private_modes.insert(mode, on);
            },
            _ if TRACKED_PRIVATE.iter().any(|&(m, _)| m == mode) => {
                self.vt.private_modes.insert(mode, on);
            },
            _ => debug!("Ignoring unknown mode {mode}"),
        }
    }

    /// xterm's ?47 and ?1047: switch screens, the cursor staying where it
    /// is (unlike ?1049, which saves and restores it). The alternate screen
    /// keeps its contents; ?1047 clears it on the way out.
    pub(crate) fn alt_screen(&mut self, on: bool, clear_on_exit: bool) {
        let alt = self.mode.contains(TermMode::ALT_SCREEN);
        if on == alt {
            return;
        }
        if !on && clear_on_exit {
            self.grid.reset_region(..);
        }
        let cursor = self.grid.cursor.clone();
        std::mem::swap(&mut self.keyboard_mode_stack, &mut self.inactive_keyboard_mode_stack);
        let keyboard = self
            .keyboard_mode_stack
            .last()
            .copied()
            .unwrap_or(crate::vte::ansi::KeyboardModes::NO_MODE);
        self.set_keyboard_mode(
            keyboard.into(),
            crate::vte::ansi::KeyboardModesApplyBehavior::Replace,
        );
        std::mem::swap(&mut self.grid, &mut self.inactive_grid);
        self.mode ^= TermMode::ALT_SCREEN;
        self.grid.cursor = cursor;
        self.selection = None;
        self.mark_fully_damaged();
    }

    /// DECRQM state of a mode upstream does not name; `None` = unsupported.
    pub(crate) fn extended_private_mode(&self, mode: u16) -> Option<bool> {
        match mode {
            5 => Some(self.vt.reverse_screen),
            40 => Some(self.vt.allow_column_switch),
            45 => Some(self.vt.reverse_wrap),
            1045 => Some(self.vt.reverse_wrap_extended),
            69 => Some(self.vt.declrmm),
            95 => Some(self.vt.no_clear_on_column),
            47 | 1047 => Some(self.mode.contains(TermMode::ALT_SCREEN)),
            1048 => Some(false),
            66 => Some(self.mode.contains(TermMode::APP_KEYPAD)),
            _ => {
                let default = TRACKED_PRIVATE.iter().find(|&&(m, _)| m == mode)?.1;
                Some(*self.vt.private_modes.get(&mode).unwrap_or(&default))
            },
        }
    }

    /// SM/RM of an ANSI mode upstream does not name.
    pub(crate) fn set_extended_ansi_mode(&mut self, mode: u16, on: bool) {
        if TRACKED_ANSI.iter().any(|&(m, _)| m == mode) {
            self.vt.ansi_modes.insert(mode, on);
        } else {
            debug!("Ignoring unknown mode {mode}");
        }
    }

    pub(crate) fn extended_ansi_mode(&self, mode: u16) -> Option<bool> {
        let default = TRACKED_ANSI.iter().find(|&&(m, _)| m == mode)?.1;
        Some(*self.vt.ansi_modes.get(&mode).unwrap_or(&default))
    }

    /// XTWINOPS: titles and reports Term can answer, the rest to the host.
    pub(crate) fn do_window_op(&mut self, given: &[Option<u16>]) {
        let params: Vec<u16> = given.iter().map(|p| p.unwrap_or(0)).collect();
        let p1 = params.get(1).copied().unwrap_or(0);
        let clean = |s: &Option<String>| -> String {
            s.as_deref().unwrap_or("").chars().filter(|c| !c.is_control()).collect()
        };
        match params.first().copied().unwrap_or(0) {
            // icon label / window title
            20 => {
                let icon = clean(&self.vt.icon.clone().or_else(|| self.title.clone()));
                self.reply(format!("\x1b]L{icon}\x1b\\"));
            },
            21 => {
                let title = clean(&self.title);
                self.reply(format!("\x1b]l{title}\x1b\\"));
            },
            // push / pop (xterm): one stack; every push saves both names,
            // every pop takes one entry and restores 0 both, 1 the icon
            // name, 2 the window title
            22 => {
                if self.vt.name_stack.len() >= TITLE_STACK_MAX_DEPTH {
                    self.vt.name_stack.remove(0);
                }
                self.vt.name_stack.push((self.title.clone(), self.vt.icon.clone()));
            },
            23 => {
                if let Some((title, icon)) = self.vt.name_stack.pop() {
                    if p1 != 1 {
                        use crate::vte::ansi::Handler;
                        self.set_title(title);
                    }
                    if p1 != 2 {
                        self.vt.icon = icon;
                    }
                }
            },
            // character cell size in pixels
            16 => {
                self.event_proxy.send_event(Event::TextAreaSizeRequest(std::sync::Arc::new(
                    |size| format!("\x1b[6;{};{}t", size.cell_height, size.cell_width),
                )));
            },
            // text area in characters
            18 => {
                let text = format!("\x1b[8;{};{}t", self.screen_lines(), self.columns());
                self.reply(text);
            },
            // resize in characters: omitted keeps, 0 (the screen) only the
            // host knows
            8 => {
                let rows = match given.get(1).copied().flatten() {
                    None => Some(self.screen_lines()),
                    Some(0) => None,
                    Some(r) => Some(r as usize),
                };
                let cols = match given.get(2).copied().flatten() {
                    None => Some(self.columns()),
                    Some(0) => None,
                    Some(c) => Some(c as usize),
                };
                if let (Some(rows), Some(cols)) = (rows, cols) {
                    self.request_resize(rows, cols);
                }
                self.event_proxy.send_event(Event::WindowOp(given.to_vec()));
            },
            // DECSLPP: a line count of 24 or more resizes
            n if n >= 24 => {
                self.vt.page_lines = Some(n);
                self.request_resize(n as usize, self.columns());
                self.event_proxy.send_event(Event::WindowOp(vec![Some(8), Some(n), None]));
            },
            _ => self.event_proxy.send_event(Event::WindowOp(given.to_vec())),
        }
    }

    /// Ask the host for a grid of `rows`×`cols` (bounded to something sane).
    pub(crate) fn request_resize(&mut self, rows: usize, cols: usize) {
        let rows = rows.clamp(2, 1000);
        let cols = cols.clamp(2, 1000);
        self.vt.pending_resize = Some((rows, cols));
    }

    /// DECCRA: copy a rectangle (pages are ignored: there is one).
    pub(crate) fn do_copy_rect(&mut self, src: Rect, top: u16, left: u16) {
        let Some(src) = self.resolve_rect(src) else {
            return;
        };
        let (row0, row_max, col0, col_max) = if self.mode.contains(TermMode::ORIGIN) {
            let (t, b) = self.tb();
            let (l, r) = self.lr();
            (t, b, l, r)
        } else {
            (0, self.screen_lines() - 1, 0, self.columns() - 1)
        };
        let dst_top = row0 + cmp::max(top, 1) as usize - 1;
        let dst_left = col0 + cmp::max(left, 1) as usize - 1;
        if dst_top > row_max || dst_left > col_max {
            return;
        }
        let height = cmp::min(src.bottom - src.top, row_max - dst_top) + 1;
        let width = cmp::min(src.right - src.left, col_max - dst_left) + 1;
        let copy: Vec<Vec<Cell>> = (0..height)
            .map(|r| {
                (0..width)
                    .map(|c| self.grid[Line((src.top + r) as i32)][Column(src.left + c)].clone())
                    .collect()
            })
            .collect();
        for (r, row) in copy.into_iter().enumerate() {
            for (c, cell) in row.into_iter().enumerate() {
                self.grid[Line((dst_top + r) as i32)][Column(dst_left + c)] = cell;
            }
        }
        self.mark_fully_damaged();
    }

    /// DECFRA: fill a rectangle with a character in the current rendition.
    pub(crate) fn do_fill_rect(&mut self, c: char, rect: Rect) {
        let code = c as u32;
        if !((32..=126).contains(&code) || (160..=255).contains(&code)) {
            return;
        }
        let Some(area) = self.resolve_rect(rect) else {
            return;
        };
        let c = self.grid.cursor.charsets[self.active_charset].map(c);
        let mut cell = self.grid.cursor.template.clone();
        cell.c = c;
        cell.flags.remove(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER | Flags::WRAPLINE);
        for line in area.top..=area.bottom {
            for col in area.left..=area.right {
                self.grid[Line(line as i32)][Column(col)] = cell.clone();
            }
        }
        self.mark_fully_damaged();
    }

    /// DECCARA / DECRARA over a rectangle or (DECSACE stream) the text
    /// between two corners.
    pub(crate) fn do_change_rect_attrs(&mut self, rect: Rect, attrs: &[u16], reverse: bool) {
        let Some(area) = self.resolve_rect(rect) else {
            return;
        };
        let attrs: Vec<u16> = if attrs.is_empty() { vec![0] } else { attrs.to_vec() };
        let last_col = self.columns() - 1;
        for line in area.top..=area.bottom {
            let (from, to) = if self.vt.rect_extent || area.top == area.bottom {
                (area.left, area.right)
            } else if line == area.top {
                (area.left, last_col)
            } else if line == area.bottom {
                (0, area.right)
            } else {
                (0, last_col)
            };
            for col in from..=to {
                let cell = &mut self.grid[Line(line as i32)][Column(col)];
                for &a in &attrs {
                    let flag = match a {
                        0 => Flags::BOLD | Flags::UNDERLINE | Flags::BLINK | Flags::INVERSE,
                        1 | 22 => Flags::BOLD,
                        4 | 24 => Flags::UNDERLINE,
                        5 | 25 => Flags::BLINK,
                        7 | 27 => Flags::INVERSE,
                        8 | 28 => Flags::HIDDEN,
                        _ => Flags::empty(),
                    };
                    if reverse {
                        if !(22..=28).contains(&a) {
                            cell.flags.toggle(flag);
                        }
                    } else if a == 0 || (22..=28).contains(&a) {
                        cell.flags.remove(flag);
                    } else {
                        cell.flags.insert(flag);
                    }
                }
            }
        }
        self.mark_fully_damaged();
    }

    /// BS and CUB: back `n` columns. Under DECAWM with reverse wraparound
    /// the motion continues at the right margin of the line above: ?45
    /// (xterm 383+) only onto a line that soft-wrapped, ?1045 onto any line
    /// and from the top margin to the bottom one. Otherwise it stops at the
    /// left margin (column 1 when the cursor started left of it). A pending
    /// wrap is consumed first.
    pub(crate) fn cursor_back(&mut self, n: usize) {
        let reverse = (self.vt.reverse_wrap || self.vt.reverse_wrap_extended)
            && self.mode.contains(TermMode::LINE_WRAP);
        let mut n = n;
        if self.grid.cursor.input_needs_wrap {
            self.grid.cursor.input_needs_wrap = false;
            if reverse {
                n = n.saturating_sub(1);
            }
        }
        if n == 0 {
            return;
        }
        let col = self.grid.cursor.point.column.0;
        let row = self.grid.cursor.point.line.0 as usize;
        let (margin, right) = self.lr();
        let left = if col < margin { 0 } else { margin };
        if !reverse {
            let new = cmp::max(col.saturating_sub(n), left);
            self.damage.damage_line(row, new, col);
            self.grid.cursor.point.column = Column(new);
            return;
        }
        self.damage.damage_line(row, 0, col);
        if !self.vt.reverse_wrap_extended {
            // ?45 (xterm 383+): back only onto lines that soft-wrapped into
            // the next one, never past the top margin
            let top = if self.cursor_in_tb() { self.tb().0 } else { 0 };
            let (mut row, mut col, mut n) = (row, col, n);
            while n > 0 {
                if col > left {
                    let step = cmp::min(n, col - left);
                    col -= step;
                    n -= step;
                    continue;
                }
                let above = Line(row as i32 - 1);
                if row == top
                    || row == 0
                    || !self.grid[above][Column(right)].flags.contains(Flags::WRAPLINE)
                {
                    break;
                }
                row -= 1;
                col = right;
                n -= 1;
            }
            self.damage.damage_line(row, col, self.columns() - 1);
            self.grid.cursor.point.line = Line(row as i32);
            self.grid.cursor.point.column = Column(col);
            return;
        }
        // ?1045: anywhere, wrapping from the top margin to the bottom one
        let in_row = (right - left + 1) as i64;
        let (top, bottom) =
            if self.cursor_in_tb() { self.tb() } else { (0, self.screen_lines() - 1) };
        let length = in_row * (bottom - top + 1) as i64;
        let offset =
            (in_row * (row - top) as i64 + col as i64 - n as i64 - left as i64).rem_euclid(length);
        let new_row = (top as i64 + offset / in_row) as usize;
        let new_col = left + (offset % in_row) as usize;
        self.damage.damage_line(new_row, new_col, self.columns() - 1);
        self.grid.cursor.point.line = Line(new_row as i32);
        self.grid.cursor.point.column = Column(new_col);
    }

    /// DECRQM state of any DEC private mode, for XTSAVE.
    pub(crate) fn private_mode_state(&self, mode: u16) -> Option<bool> {
        let m = &self.mode;
        Some(match mode {
            1 => m.contains(TermMode::APP_CURSOR),
            6 => m.contains(TermMode::ORIGIN),
            7 => m.contains(TermMode::LINE_WRAP),
            25 => m.contains(TermMode::SHOW_CURSOR),
            1000 => m.contains(TermMode::MOUSE_REPORT_CLICK),
            1002 => m.contains(TermMode::MOUSE_DRAG),
            1003 => m.contains(TermMode::MOUSE_MOTION),
            1004 => m.contains(TermMode::FOCUS_IN_OUT),
            1005 => m.contains(TermMode::UTF8_MOUSE),
            1006 => m.contains(TermMode::SGR_MOUSE),
            1007 => m.contains(TermMode::ALTERNATE_SCROLL),
            1049 => m.contains(TermMode::ALT_SCREEN),
            2004 => m.contains(TermMode::BRACKETED_PASTE),
            3 => self.vt.columns_132,
            _ => return self.extended_private_mode(mode),
        })
    }

    /// DECSTR soft reset.
    pub(crate) fn do_soft_reset(&mut self) {
        use crate::vte::ansi::{CharsetIndex, StandardCharset};
        self.mode.insert(TermMode::SHOW_CURSOR);
        self.mode.remove(
            TermMode::INSERT | TermMode::ORIGIN | TermMode::APP_KEYPAD | TermMode::APP_CURSOR,
        );
        self.vt.private_modes.remove(&66);
        self.vt.reverse_wrap = false;
        self.vt.reverse_wrap_extended = false;
        self.vt.declrmm = false;
        self.vt.iso_protect = false;
        self.vt.iso_mode = false;
        self.scroll_region = Line(0)..Line(self.screen_lines() as i32);
        self.vt.left = 0;
        self.vt.right = self.columns() - 1;
        self.active_charset = CharsetIndex::G0;
        for index in [CharsetIndex::G0, CharsetIndex::G1, CharsetIndex::G2, CharsetIndex::G3] {
            self.grid.cursor.charsets[index] = StandardCharset::Ascii;
        }
        let template = &mut self.grid.cursor.template;
        template.fg = Color::Named(NamedColor::Foreground);
        template.bg = Color::Named(NamedColor::Background);
        template.flags = Flags::empty();
        // DECSC state: home, default rendition
        let mut saved = self.grid.cursor.clone();
        saved.point = crate::index::Point::new(Line(0), Column(0));
        saved.input_needs_wrap = false;
        saved.origin = false;
        self.grid.saved_cursor = saved;
        self.mark_fully_damaged();
    }

    /// DECRQPSR: DECCIR (1) or DECTABSR (2).
    pub(crate) fn do_presentation_report(&mut self, n: u16) {
        match n {
            1 => {
                let cursor = &self.grid.cursor;
                let flags = cursor.template.flags;
                let point = cursor.point;
                let mut rend = 0x40u8;
                for (flag, bit) in [
                    (Flags::BOLD, 1),
                    (Flags::UNDERLINE, 2),
                    (Flags::BLINK, 4),
                    (Flags::INVERSE, 8),
                ] {
                    if flags.contains(flag) {
                        rend |= bit;
                    }
                }
                let att = 0x40u8 | u8::from(flags.contains(Flags::PROTECTED));
                let sflag = 0x40u8
                    | u8::from(self.mode.contains(TermMode::ORIGIN))
                    | if cursor.input_needs_wrap { 8 } else { 0 };
                let gl = self.active_charset as u8;
                let designators: String = [
                    crate::vte::ansi::CharsetIndex::G0,
                    crate::vte::ansi::CharsetIndex::G1,
                    crate::vte::ansi::CharsetIndex::G2,
                    crate::vte::ansi::CharsetIndex::G3,
                ]
                .iter()
                .map(|&i| designator(cursor.charsets[i]))
                .collect();
                let text = format!(
                    "\x1bP1$u{};{};1;{};{};{};{};2;@;{}\x1b\\",
                    point.line.0 + 1,
                    point.column.0 + 1,
                    rend as char,
                    att as char,
                    sflag as char,
                    gl,
                    designators
                );
                self.reply(text);
            },
            2 => {
                let stops: Vec<String> = (0..self.columns())
                    .filter(|&c| self.tabs[Column(c)])
                    .map(|c| (c + 1).to_string())
                    .collect();
                self.reply(format!("\x1bP2$u{}\x1b\\", stops.join("/")));
            },
            _ => self.ignore("DECRQPSR"),
        }
    }

    /// DECRSPS: restore what DECCIR (1) or DECTABSR (2) reported.
    pub(crate) fn restore_presentation(&mut self, kind: u16, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        match kind {
            1 => {
                let fields: Vec<&str> = text.split(';').collect();
                let num = |i: usize| fields.get(i).and_then(|f| f.parse::<usize>().ok());
                if let (Some(row), Some(col)) = (num(0), num(1)) {
                    let row = cmp::min(cmp::max(row, 1), self.screen_lines()) - 1;
                    let col = cmp::min(cmp::max(col, 1), self.columns()) - 1;
                    self.grid.cursor.point =
                        crate::index::Point::new(Line(row as i32), Column(col));
                }
                let byte = |i: usize| fields.get(i).and_then(|f| f.bytes().next());
                if let Some(rend) = byte(3) {
                    let flags = &mut self.grid.cursor.template.flags;
                    flags.set(Flags::BOLD, rend & 1 != 0);
                    flags.set(Flags::UNDERLINE, rend & 2 != 0);
                    flags.set(Flags::BLINK, rend & 4 != 0);
                    flags.set(Flags::INVERSE, rend & 8 != 0);
                }
                if let Some(att) = byte(4) {
                    self.grid.cursor.template.flags.set(Flags::PROTECTED, att & 1 != 0);
                }
                if let Some(flag) = byte(5) {
                    self.mode.set(TermMode::ORIGIN, flag & 1 != 0);
                    self.grid.cursor.input_needs_wrap = flag & 8 != 0;
                }
            },
            2 => {
                self.tabs.clear_all();
                for stop in text.split('/').filter_map(|s| s.parse::<usize>().ok()) {
                    if stop >= 1 && stop <= self.columns() {
                        self.tabs[Column(stop - 1)] = true;
                    }
                }
            },
            _ => self.ignore("DECRSPS"),
        }
    }

    /// DECRQTSR: the terminal state report is opaque; reply with an empty
    /// one (1) or the color table (2).
    pub(crate) fn do_terminal_state_report(&mut self, n: u16) {
        match n {
            1 => self.reply(String::from("\x1bP1$s\x1b\\")),
            2 => self.reply(String::from("\x1bP2$s\x1b\\")),
            _ => self.ignore("DECRQTSR"),
        }
    }

    /// DEC locator: enable (DECELR), event selection (DECSLE), position
    /// request (DECRQLP).
    pub(crate) fn do_locator(&mut self, op: char, params: &[u16]) {
        match op {
            'z' => {
                let mode = params.first().copied().unwrap_or(0);
                self.vt.locator = mode;
                self.vt.locator_one_shot = mode == 2;
            },
            '{' => {},
            '|' => {
                let text = match (self.vt.locator, self.vt.locator_pos) {
                    (0, _) | (_, None) => String::from("\x1b[0&w"),
                    (_, Some((line, col))) => {
                        format!("\x1b[1;0;{};{};1&w", line + 1, col + 1)
                    },
                };
                if self.vt.locator_one_shot {
                    self.vt.locator = 0;
                }
                self.reply(text);
            },
            _ => self.ignore("locator"),
        }
    }
}

/// The SCS final byte designating `charset`.
fn designator(charset: crate::vte::ansi::StandardCharset) -> char {
    use crate::vte::ansi::StandardCharset as C;
    match charset {
        C::Ascii => 'B',
        C::SpecialCharacterAndLineDrawing => '0',
        C::UnitedKingdom => 'A',
        C::Dutch => '4',
        C::Finnish => 'C',
        C::French => 'R',
        C::FrenchCanadian => 'Q',
        C::German => 'K',
        C::Italian => 'Y',
        C::NorwegianDanish => 'E',
        C::Spanish => 'Z',
        C::Swedish => 'H',
        C::Swiss => '=',
        C::DecSupplemental => '<',
        C::DecTechnical => '>',
    }
}

impl<T> Term<T> {
    /// Where the pointer is, in cells (the DEC locator reads it).
    pub fn set_locator_position(&mut self, position: Option<(usize, usize)>) {
        self.vt.locator_pos = position;
    }
}
