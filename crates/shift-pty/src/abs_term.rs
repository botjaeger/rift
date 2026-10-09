// GENERATED delegation (see M3 notes): forwards every vte Handler method to
// the wrapped Term, intercepting the few that can push lines into scrollback
// history so we can keep a monotone absolute-line counter (`scrolled`).
// Generated against vte 0.15 — regenerate if the Handler trait changes.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{
    Attr, CharsetIndex, ClearMode, CursorShape, CursorStyle, Handler, Hyperlink, KeyboardModes,
    KeyboardModesApplyBehavior, LineClearMode, Mode, ModifyOtherKeys, PrivateMode, Rgb,
    ScpCharPath, ScpUpdateMode, StandardCharset, TabulationClearMode,
};

/// Wraps a Term to observe lines entering scrollback history.
///
/// `scrolled` counts every line ever pushed into history (including ones later
/// trimmed), so `scrolled + screen_row` is a stable absolute line index and
/// `abs - scrolled` converts back to a grid Line.
pub struct AbsTerm<'a, L: EventListener> {
    pub term: &'a mut Term<L>,
    pub scrolled: &'a mut u64,
    /// Configured scrollback cap (the grid's history limit), restored after
    /// every tracked operation.
    pub scrollback_cap: usize,
    /// Set when ED3 (ClearMode::Saved) ran on the primary screen — only the
    /// `clear` command emits it (terminfo E3), never full-screen TUIs, so the
    /// reader thread uses it to wipe the block list Warp-style.
    pub saw_clear_saved: bool,
}

impl<L: EventListener> AbsTerm<'_, L> {
    fn primary(&self) -> bool {
        !self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Count the lines `f` pushes into primary history. At (or near) the cap
    /// alacritty pushes and trims in one rotation, so history_size() alone
    /// cannot see it: lift the limit by a screen first (one operation pushes
    /// at most a screen of lines), measure the exact growth, then trim back
    /// to the cap. The alternate screen has no history and pushes nothing.
    fn track(&mut self, f: impl FnOnce(&mut Term<L>)) {
        if !self.primary() {
            f(self.term);
            return;
        }
        let grid = self.term.grid_mut();
        let before = grid.history_size();
        let headroom = before.max(self.scrollback_cap) + grid.screen_lines();
        grid.update_history(headroom);
        f(self.term);
        let grid = self.term.grid_mut();
        *self.scrolled += grid.history_size().saturating_sub(before) as u64;
        grid.update_history(self.scrollback_cap);
    }
}

impl<L: EventListener> Handler for AbsTerm<'_, L> {
    fn linefeed(&mut self) {
        self.track(|t| t.linefeed());
    }

    fn newline(&mut self) {
        self.track(|t| t.newline());
    }

    fn scroll_up(&mut self, count: usize) {
        self.track(|t| t.scroll_up(count));
    }

    fn clear_screen(&mut self, mode: ClearMode) {
        if self.primary() && matches!(mode, ClearMode::Saved) {
            // `clear` (terminfo E3): Warp-style soft clear. Flag it for the
            // reader thread and do NOT forward — alacritty would drop the
            // saved lines, but scrollback must stay reachable above the fold.
            self.saw_clear_saved = true;
            return;
        }
        // alacritty scrolls the viewport into history on ClearMode::All
        self.track(|t| t.clear_screen(mode));
    }

    fn reset_state(&mut self) {
        self.term.reset_state();
    }

    fn set_title(&mut self, a0: Option<String>) {
        self.term.set_title(a0);
    }

    fn set_cursor_style(&mut self, a0: Option<CursorStyle>) {
        self.term.set_cursor_style(a0);
    }

    fn set_cursor_shape(&mut self, a0: CursorShape) {
        self.term.set_cursor_shape(a0);
    }

    fn input(&mut self, a0: char) {
        // only a char at the last column can wrap (a pending wrap, or a wide
        // glyph that does not fit) and so scroll a line into history; the
        // rest skip the tracking cost
        let grid = self.term.grid();
        let cursor = &grid.cursor;
        if cursor.input_needs_wrap || cursor.point.column.0 + 1 >= grid.columns() {
            self.track(|t| t.input(a0));
        } else {
            self.term.input(a0);
        }
    }

    fn goto(&mut self, a0: i32, a1: usize) {
        self.term.goto(a0, a1);
    }

    fn goto_line(&mut self, a0: i32) {
        self.term.goto_line(a0);
    }

    fn goto_col(&mut self, a0: usize) {
        self.term.goto_col(a0);
    }

    fn insert_blank(&mut self, a0: usize) {
        self.term.insert_blank(a0);
    }

    fn move_up(&mut self, a0: usize) {
        self.term.move_up(a0);
    }

    fn move_down(&mut self, a0: usize) {
        self.term.move_down(a0);
    }

    fn identify_terminal(&mut self, a0: Option<char>) {
        self.term.identify_terminal(a0);
    }

    fn device_status(&mut self, a0: usize) {
        self.term.device_status(a0);
    }

    fn move_forward(&mut self, a0: usize) {
        self.term.move_forward(a0);
    }

    fn move_backward(&mut self, a0: usize) {
        self.term.move_backward(a0);
    }

    fn move_down_and_cr(&mut self, a0: usize) {
        self.term.move_down_and_cr(a0);
    }

    fn move_up_and_cr(&mut self, a0: usize) {
        self.term.move_up_and_cr(a0);
    }

    fn put_tab(&mut self, a0: u16) {
        self.term.put_tab(a0);
    }

    fn backspace(&mut self) {
        self.term.backspace();
    }

    fn carriage_return(&mut self) {
        self.term.carriage_return();
    }

    fn bell(&mut self) {
        self.term.bell();
    }

    fn substitute(&mut self) {
        self.term.substitute();
    }

    fn set_horizontal_tabstop(&mut self) {
        self.term.set_horizontal_tabstop();
    }

    fn scroll_down(&mut self, a0: usize) {
        self.term.scroll_down(a0);
    }

    fn insert_blank_lines(&mut self, a0: usize) {
        self.term.insert_blank_lines(a0);
    }

    fn delete_lines(&mut self, a0: usize) {
        // on the top row of a full-screen region this scrolls into history
        self.track(|t| t.delete_lines(a0));
    }

    fn erase_chars(&mut self, a0: usize) {
        self.term.erase_chars(a0);
    }

    fn delete_chars(&mut self, a0: usize) {
        self.term.delete_chars(a0);
    }

    fn move_backward_tabs(&mut self, a0: u16) {
        self.term.move_backward_tabs(a0);
    }

    fn move_forward_tabs(&mut self, a0: u16) {
        self.term.move_forward_tabs(a0);
    }

    fn save_cursor_position(&mut self) {
        self.term.save_cursor_position();
    }

    fn restore_cursor_position(&mut self) {
        self.term.restore_cursor_position();
    }

    fn clear_line(&mut self, a0: LineClearMode) {
        self.term.clear_line(a0);
    }

    fn clear_tabs(&mut self, a0: TabulationClearMode) {
        self.term.clear_tabs(a0);
    }

    fn set_tabs(&mut self, a0: u16) {
        self.term.set_tabs(a0);
    }

    fn reverse_index(&mut self) {
        self.term.reverse_index();
    }

    fn terminal_attribute(&mut self, a0: Attr) {
        self.term.terminal_attribute(a0);
    }

    fn set_mode(&mut self, a0: Mode) {
        self.term.set_mode(a0);
    }

    fn unset_mode(&mut self, a0: Mode) {
        self.term.unset_mode(a0);
    }

    fn report_mode(&mut self, a0: Mode) {
        self.term.report_mode(a0);
    }

    fn set_private_mode(&mut self, a0: PrivateMode) {
        self.term.set_private_mode(a0);
    }

    fn unset_private_mode(&mut self, a0: PrivateMode) {
        self.term.unset_private_mode(a0);
    }

    fn report_private_mode(&mut self, a0: PrivateMode) {
        self.term.report_private_mode(a0);
    }

    fn set_scrolling_region(&mut self, a0: usize, a1: Option<usize>) {
        self.term.set_scrolling_region(a0, a1);
    }

    fn set_keypad_application_mode(&mut self) {
        self.term.set_keypad_application_mode();
    }

    fn unset_keypad_application_mode(&mut self) {
        self.term.unset_keypad_application_mode();
    }

    fn set_active_charset(&mut self, a0: CharsetIndex) {
        self.term.set_active_charset(a0);
    }

    fn configure_charset(&mut self, a0: CharsetIndex, a1: StandardCharset) {
        self.term.configure_charset(a0, a1);
    }

    fn set_color(&mut self, a0: usize, a1: Rgb) {
        self.term.set_color(a0, a1);
    }

    fn dynamic_color_sequence(&mut self, a0: String, a1: usize, a2: &str) {
        self.term.dynamic_color_sequence(a0, a1, a2);
    }

    fn reset_color(&mut self, a0: usize) {
        self.term.reset_color(a0);
    }

    fn clipboard_store(&mut self, a0: u8, a1: &[u8]) {
        self.term.clipboard_store(a0, a1);
    }

    fn clipboard_load(&mut self, a0: u8, a1: &str) {
        self.term.clipboard_load(a0, a1);
    }

    fn decaln(&mut self) {
        self.term.decaln();
    }

    fn push_title(&mut self) {
        self.term.push_title();
    }

    fn pop_title(&mut self) {
        self.term.pop_title();
    }

    fn text_area_size_pixels(&mut self) {
        self.term.text_area_size_pixels();
    }

    fn text_area_size_chars(&mut self) {
        self.term.text_area_size_chars();
    }

    fn set_hyperlink(&mut self, a0: Option<Hyperlink>) {
        self.term.set_hyperlink(a0);
    }

    fn report_keyboard_mode(&mut self) {
        self.term.report_keyboard_mode();
    }

    fn push_keyboard_mode(&mut self, a0: KeyboardModes) {
        self.term.push_keyboard_mode(a0);
    }

    fn pop_keyboard_modes(&mut self, a0: u16) {
        self.term.pop_keyboard_modes(a0);
    }

    fn set_keyboard_mode(&mut self, a0: KeyboardModes, a1: KeyboardModesApplyBehavior) {
        self.term.set_keyboard_mode(a0, a1);
    }

    fn set_modify_other_keys(&mut self, a0: ModifyOtherKeys) {
        self.term.set_modify_other_keys(a0);
    }

    fn report_modify_other_keys(&mut self) {
        self.term.report_modify_other_keys();
    }

    fn set_scp(&mut self, a0: ScpCharPath, a1: ScpUpdateMode) {
        self.term.set_scp(a0, a1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::Event;
    use alacritty_terminal::term::{Config as TermConfig, test::TermSize};
    use alacritty_terminal::vte;

    struct Noop;
    impl EventListener for Noop {
        fn send_event(&self, _: Event) {}
    }

    #[test]
    fn ed3_sets_saw_clear_saved() {
        let size = TermSize::new(80, 24);
        let mut term = Term::new(TermConfig::default(), &size, Noop);
        let mut scrolled = 0u64;
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        let mut abs = AbsTerm {
            term: &mut term,
            scrolled: &mut scrolled,
            scrollback_cap: 10_000,
            saw_clear_saved: false,
        };
        // exact byte sequence /usr/bin/clear emits for xterm-256color
        parser.advance(&mut abs, b"\x1b[3J\x1b[H\x1b[2J");
        assert!(abs.saw_clear_saved);
    }

    /// `scrolled` delta of `seq` on a 5×3 grid after four lines filled
    /// history up to `cap`.
    fn delta(cap: usize, seq: &str) -> u64 {
        let size = TermSize::new(5, 3);
        let config = TermConfig {
            scrolling_history: cap,
            ..TermConfig::default()
        };
        let mut term = Term::new(config, &size, Noop);
        let mut scrolled = 0u64;
        let mut parser: vte::ansi::Processor = vte::ansi::Processor::new();
        let mut run = |term: &mut Term<Noop>, scrolled: &mut u64, bytes: &[u8]| {
            let mut abs = AbsTerm {
                term,
                scrolled,
                scrollback_cap: cap,
                saw_clear_saved: false,
            };
            parser.advance(&mut abs, bytes);
        };
        run(&mut term, &mut scrolled, b"one\r\ntwo\r\nthree\r\nfour\r\n");
        assert_eq!(scrolled, 2, "fill, cap {cap}");
        assert_eq!(term.grid().history_size(), cap.min(2));
        let before = scrolled;
        run(&mut term, &mut scrolled, seq.as_bytes());
        // the grid never keeps more history than its cap
        assert!(term.grid().history_size() <= cap);
        scrolled - before
    }

    #[test]
    fn pushes_counted_exactly_below_and_at_the_cap() {
        for cap in [0, 1, 2, 100] {
            for (name, seq, want) in [
                ("ordinary wrap", "abcdeX", 1),
                ("combining mark at the margin", "abcde\u{301}", 0),
                ("wide glyph early wrap", "abcd界", 1),
                ("delete lines on the top row", "\x1b[1;1H\x1b[M", 1),
                ("partial scroll region", "\x1b[2;3r\x1b[S", 0),
                ("huge scroll up", "\x1b[999S", 3),
                ("linefeeds crossing the cap", "a\r\nb\r\nc\r\nd\r\n", 4),
                ("clear screen (two used rows)", "\x1b[H\x1b[2J", 2),
            ] {
                assert_eq!(delta(cap, seq), want, "{name}, cap {cap}");
            }
        }
    }

    #[test]
    fn alternate_screen_pushes_nothing() {
        for cap in [0, 2, 100] {
            assert_eq!(delta(cap, "\x1b[?1049h\x1b[999S\r\n\r\n\r\n\r\n"), 0);
        }
    }
}
