//! Exports the `Term` type which is a high-level API for the Grid.

use std::ops::{Index, IndexMut, Range};
use std::sync::Arc;
use std::{cmp, mem, ptr, slice, str};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as Base64;
use bitflags::bitflags;
use log::{debug, trace};

use crate::event::{Event, EventListener};
use crate::grid::{Dimensions, Grid, GridIterator, Scroll};
use crate::index::{self, Boundary, Column, Direction, Line, Point, Side};
use crate::selection::{Selection, SelectionRange, SelectionType};
use crate::term::cell::{Cell, Flags, LineLength};
use crate::term::color::Colors;
use crate::vi_mode::{ViModeCursor, ViMotion};
use crate::vte::ansi::{
    self, Attr, CharsetIndex, Color, CursorShape, CursorStyle, Handler, Hyperlink, KeyboardModes,
    KeyboardModesApplyBehavior, NamedColor, NamedMode, NamedPrivateMode, PrivateMode, Rgb,
    StandardCharset,
};

pub mod cell;
pub mod color;
pub mod search;
mod vt420;
pub mod width;
mod width_tables;

/// Minimum number of columns.
///
/// A minimum of 2 is necessary to hold fullwidth unicode characters.
pub const MIN_COLUMNS: usize = 2;

/// Minimum number of visible lines.
pub const MIN_SCREEN_LINES: usize = 1;

/// Max size of the window title stack.
const TITLE_STACK_MAX_DEPTH: usize = 4096;

/// Default semantic escape characters.
pub const SEMANTIC_ESCAPE_CHARS: &str = ",│`|:\"' ()[]{}<>\t";

/// Max size of the keyboard modes.
const KEYBOARD_MODE_STACK_MAX_DEPTH: usize = TITLE_STACK_MAX_DEPTH;

/// Default tab interval, corresponding to terminfo `it` value.
const INITIAL_TABSTOPS: usize = 8;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TermMode: u32 {
        const NONE                    = 0;
        const SHOW_CURSOR             = 1;
        const APP_CURSOR              = 1 << 1;
        const APP_KEYPAD              = 1 << 2;
        const MOUSE_REPORT_CLICK      = 1 << 3;
        const BRACKETED_PASTE         = 1 << 4;
        const SGR_MOUSE               = 1 << 5;
        const MOUSE_MOTION            = 1 << 6;
        const LINE_WRAP               = 1 << 7;
        const LINE_FEED_NEW_LINE      = 1 << 8;
        const ORIGIN                  = 1 << 9;
        const INSERT                  = 1 << 10;
        const FOCUS_IN_OUT            = 1 << 11;
        const ALT_SCREEN              = 1 << 12;
        const MOUSE_DRAG              = 1 << 13;
        const UTF8_MOUSE              = 1 << 14;
        const ALTERNATE_SCROLL        = 1 << 15;
        const VI                      = 1 << 16;
        const URGENCY_HINTS           = 1 << 17;
        const DISAMBIGUATE_ESC_CODES  = 1 << 18;
        const REPORT_EVENT_TYPES      = 1 << 19;
        const REPORT_ALTERNATE_KEYS   = 1 << 20;
        const REPORT_ALL_KEYS_AS_ESC  = 1 << 21;
        const REPORT_ASSOCIATED_TEXT  = 1 << 22;
        const MOUSE_MODE              = Self::MOUSE_REPORT_CLICK.bits() | Self::MOUSE_MOTION.bits() | Self::MOUSE_DRAG.bits();
        const KITTY_KEYBOARD_PROTOCOL = Self::DISAMBIGUATE_ESC_CODES.bits()
                                      | Self::REPORT_EVENT_TYPES.bits()
                                      | Self::REPORT_ALTERNATE_KEYS.bits()
                                      | Self::REPORT_ALL_KEYS_AS_ESC.bits()
                                      | Self::REPORT_ASSOCIATED_TEXT.bits();
         const ANY                    = u32::MAX;
    }
}

impl From<KeyboardModes> for TermMode {
    fn from(value: KeyboardModes) -> Self {
        let mut mode = Self::empty();

        let disambiguate_esc_codes = value.contains(KeyboardModes::DISAMBIGUATE_ESC_CODES);
        mode.set(TermMode::DISAMBIGUATE_ESC_CODES, disambiguate_esc_codes);

        let report_event_types = value.contains(KeyboardModes::REPORT_EVENT_TYPES);
        mode.set(TermMode::REPORT_EVENT_TYPES, report_event_types);

        let report_alternate_keys = value.contains(KeyboardModes::REPORT_ALTERNATE_KEYS);
        mode.set(TermMode::REPORT_ALTERNATE_KEYS, report_alternate_keys);

        let report_all_keys_as_esc = value.contains(KeyboardModes::REPORT_ALL_KEYS_AS_ESC);
        mode.set(TermMode::REPORT_ALL_KEYS_AS_ESC, report_all_keys_as_esc);

        let report_associated_text = value.contains(KeyboardModes::REPORT_ASSOCIATED_TEXT);
        mode.set(TermMode::REPORT_ASSOCIATED_TEXT, report_associated_text);

        mode
    }
}

impl From<TermMode> for KeyboardModes {
    fn from(value: TermMode) -> Self {
        let mut modes = Self::NO_MODE;
        let pairs = [
            (TermMode::DISAMBIGUATE_ESC_CODES, KeyboardModes::DISAMBIGUATE_ESC_CODES),
            (TermMode::REPORT_EVENT_TYPES, KeyboardModes::REPORT_EVENT_TYPES),
            (TermMode::REPORT_ALTERNATE_KEYS, KeyboardModes::REPORT_ALTERNATE_KEYS),
            (TermMode::REPORT_ALL_KEYS_AS_ESC, KeyboardModes::REPORT_ALL_KEYS_AS_ESC),
            (TermMode::REPORT_ASSOCIATED_TEXT, KeyboardModes::REPORT_ASSOCIATED_TEXT),
        ];
        for (term, keyboard) in pairs {
            modes.set(keyboard, value.contains(term));
        }
        modes
    }
}

impl Default for TermMode {
    fn default() -> TermMode {
        TermMode::SHOW_CURSOR
            | TermMode::LINE_WRAP
            | TermMode::ALTERNATE_SCROLL
            | TermMode::URGENCY_HINTS
    }
}

/// Convert a terminal point to a viewport relative point.
#[inline]
pub fn point_to_viewport(display_offset: usize, point: Point) -> Option<Point<usize>> {
    let viewport_line = point.line.0 + display_offset as i32;
    usize::try_from(viewport_line).ok().map(|line| Point::new(line, point.column))
}

/// Convert a viewport relative point to a terminal point.
#[inline]
pub fn viewport_to_point(display_offset: usize, point: Point<usize>) -> Point {
    let line = Line(point.line as i32) - display_offset;
    Point::new(line, point.column)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineDamageBounds {
    /// Damaged line number.
    pub line: usize,

    /// Leftmost damaged column.
    pub left: usize,

    /// Rightmost damaged column.
    pub right: usize,
}

impl LineDamageBounds {
    #[inline]
    pub fn new(line: usize, left: usize, right: usize) -> Self {
        Self { line, left, right }
    }

    #[inline]
    pub fn undamaged(line: usize, num_cols: usize) -> Self {
        Self { line, left: num_cols, right: 0 }
    }

    #[inline]
    pub fn reset(&mut self, num_cols: usize) {
        *self = Self::undamaged(self.line, num_cols);
    }

    #[inline]
    pub fn expand(&mut self, left: usize, right: usize) {
        self.left = cmp::min(self.left, left);
        self.right = cmp::max(self.right, right);
    }

    #[inline]
    pub fn is_damaged(&self) -> bool {
        self.left <= self.right
    }
}

/// Terminal damage information collected since the last [`Term::reset_damage`] call.
#[derive(Debug)]
pub enum TermDamage<'a> {
    /// The entire terminal is damaged.
    Full,

    /// Iterator over damaged lines in the terminal.
    Partial(TermDamageIterator<'a>),
}

/// Iterator over the terminal's viewport damaged lines.
#[derive(Clone, Debug)]
pub struct TermDamageIterator<'a> {
    line_damage: slice::Iter<'a, LineDamageBounds>,
    display_offset: usize,
}

impl<'a> TermDamageIterator<'a> {
    pub fn new(line_damage: &'a [LineDamageBounds], display_offset: usize) -> Self {
        let num_lines = line_damage.len();
        // Filter out invisible damage.
        let line_damage = &line_damage[..num_lines.saturating_sub(display_offset)];
        Self { display_offset, line_damage: line_damage.iter() }
    }
}

impl Iterator for TermDamageIterator<'_> {
    type Item = LineDamageBounds;

    fn next(&mut self) -> Option<Self::Item> {
        self.line_damage.find_map(|line| {
            line.is_damaged().then_some(LineDamageBounds::new(
                line.line + self.display_offset,
                line.left,
                line.right,
            ))
        })
    }
}

/// State of the terminal damage.
struct TermDamageState {
    /// Hint whether terminal should be damaged entirely regardless of the actual damage changes.
    full: bool,

    /// Information about damage on terminal lines.
    lines: Vec<LineDamageBounds>,

    /// Old terminal cursor point.
    last_cursor: Point,
}

impl TermDamageState {
    fn new(num_cols: usize, num_lines: usize) -> Self {
        let lines =
            (0..num_lines).map(|line| LineDamageBounds::undamaged(line, num_cols)).collect();

        Self { full: true, lines, last_cursor: Default::default() }
    }

    #[inline]
    fn resize(&mut self, num_cols: usize, num_lines: usize) {
        // Reset point, so old cursor won't end up outside of the viewport.
        self.last_cursor = Default::default();
        self.full = true;

        self.lines.clear();
        self.lines.reserve(num_lines);
        for line in 0..num_lines {
            self.lines.push(LineDamageBounds::undamaged(line, num_cols));
        }
    }

    /// Damage point inside of the viewport.
    #[inline]
    fn damage_point(&mut self, point: Point<usize>) {
        self.damage_line(point.line, point.column.0, point.column.0);
    }

    /// Expand `line`'s damage to span at least `left` to `right` column.
    #[inline]
    fn damage_line(&mut self, line: usize, left: usize, right: usize) {
        self.lines[line].expand(left, right);
    }

    /// Reset information about terminal damage.
    fn reset(&mut self, num_cols: usize) {
        self.full = false;
        self.lines.iter_mut().for_each(|line| line.reset(num_cols));
    }
}

pub struct Term<T> {
    /// Terminal focus controlling the cursor shape.
    pub is_focused: bool,

    /// Cursor for keyboard selection.
    pub vi_mode_cursor: ViModeCursor,

    pub selection: Option<Selection>,

    /// Currently active grid.
    ///
    /// Tracks the screen buffer currently in use. While the alternate screen buffer is active,
    /// this will be the alternate grid. Otherwise it is the primary screen buffer.
    grid: Grid<Cell>,

    /// Currently inactive grid.
    ///
    /// Opposite of the active grid. While the alternate screen buffer is active, this will be the
    /// primary grid. Otherwise it is the alternate screen buffer.
    inactive_grid: Grid<Cell>,

    /// Index into `charsets`, pointing to what ASCII is currently being mapped to.
    active_charset: CharsetIndex,

    /// Tabstops.
    tabs: TabStops,

    /// Mode flags.
    mode: TermMode,

    /// Scroll region.
    ///
    /// Range going from top to bottom of the terminal, indexed from the top of the viewport.
    scroll_region: Range<Line>,

    /// Modified terminal colors.
    colors: Colors,

    /// Current style of the cursor.
    cursor_style: Option<CursorStyle>,

    /// Proxy for sending events to the event loop.
    event_proxy: T,

    /// Current title of the window.
    title: Option<String>,

    /// Stack of saved window titles. When a title is popped from this stack, the `title` for the
    /// term is set.
    title_stack: Vec<Option<String>>,

    /// The stack for the keyboard modes.
    keyboard_mode_stack: Vec<KeyboardModes>,

    /// Currently inactive keyboard mode stack.
    inactive_keyboard_mode_stack: Vec<KeyboardModes>,

    /// Information about damaged cells.
    damage: TermDamageState,

    /// Config directly for the terminal.
    config: Config,

    /// VT420 / xterm state upstream does not keep (Rift).
    vt: vt420::Vt420,
}

/// Configuration options for the [`Term`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The maximum amount of scrolling history.
    pub scrolling_history: usize,

    /// Default cursor style to reset the cursor to.
    pub default_cursor_style: CursorStyle,

    /// Cursor style for Vi mode.
    pub vi_mode_cursor_style: Option<CursorStyle>,

    /// The characters which terminate semantic selection.
    ///
    /// The default value is [`SEMANTIC_ESCAPE_CHARS`].
    pub semantic_escape_chars: String,

    /// Whether to enable kitty keyboard protocol.
    pub kitty_keyboard: bool,

    /// OSC52 support mode.
    pub osc52: Osc52,

    /// Name and version XTVERSION reports (Rift).
    pub xtversion: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scrolling_history: 10000,
            semantic_escape_chars: SEMANTIC_ESCAPE_CHARS.to_owned(),
            default_cursor_style: Default::default(),
            vi_mode_cursor_style: Default::default(),
            kitty_keyboard: Default::default(),
            osc52: Default::default(),
            xtversion: format!("alacritty_terminal({})", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// OSC 52 behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(rename_all = "lowercase"))]
pub enum Osc52 {
    /// The handling of the escape sequence is disabled.
    Disabled,
    /// Only copy sequence is accepted.
    ///
    /// This option is the default as a compromise between entirely
    /// disabling it (the most secure) and allowing `paste` (the less secure).
    #[default]
    OnlyCopy,
    /// Only paste sequence is accepted.
    OnlyPaste,
    /// Both are accepted.
    CopyPaste,
}

impl<T> Term<T> {
    #[inline]
    pub fn scroll_display(&mut self, scroll: Scroll)
    where
        T: EventListener,
    {
        let old_display_offset = self.grid.display_offset();
        self.grid.scroll_display(scroll);
        self.event_proxy.send_event(Event::MouseCursorDirty);

        // Clamp vi mode cursor to the viewport.
        let viewport_start = -(self.grid.display_offset() as i32);
        let viewport_end = viewport_start + self.bottommost_line().0;
        let vi_cursor_line = &mut self.vi_mode_cursor.point.line.0;
        *vi_cursor_line = cmp::min(viewport_end, cmp::max(viewport_start, *vi_cursor_line));
        self.vi_mode_recompute_selection();

        // Damage everything if display offset changed.
        if old_display_offset != self.grid().display_offset() {
            self.mark_fully_damaged();
        }
    }

    pub fn new<D: Dimensions>(config: Config, dimensions: &D, event_proxy: T) -> Term<T> {
        let num_cols = dimensions.columns();
        let num_lines = dimensions.screen_lines();

        let history_size = config.scrolling_history;
        let grid = Grid::new(num_lines, num_cols, history_size);
        let inactive_grid = Grid::new(num_lines, num_cols, 0);

        let tabs = TabStops::new(grid.columns());

        let scroll_region = Line(0)..Line(grid.screen_lines() as i32);

        // Initialize terminal damage, covering the entire terminal upon launch.
        let damage = TermDamageState::new(num_cols, num_lines);

        Term {
            inactive_grid,
            scroll_region,
            event_proxy,
            damage,
            config,
            grid,
            tabs,
            inactive_keyboard_mode_stack: Default::default(),
            keyboard_mode_stack: Default::default(),
            active_charset: Default::default(),
            vi_mode_cursor: Default::default(),
            cursor_style: Default::default(),
            colors: color::Colors::default(),
            title_stack: Default::default(),
            is_focused: Default::default(),
            selection: Default::default(),
            title: Default::default(),
            mode: Default::default(),
            vt: vt420::Vt420::new(num_cols),
        }
    }

    /// A grid size the program requested (XTWINOPS 8, DECCOLM, DECSLPP),
    /// taken once: the host resizes the grid now and the window after.
    pub fn take_pending_resize(&mut self) -> Option<(usize, usize)> {
        self.vt.pending_resize.take()
    }

    /// DECSCNM (`CSI ? 5 h`): the program asked for a reverse-video screen.
    #[inline]
    pub fn reverse_screen(&self) -> bool {
        self.vt.reverse_screen
    }

    /// Blinking text (SGR 5/6) is on screen somewhere: the renderer needs
    /// to animate.
    #[inline]
    pub fn blink_cells(&self) -> bool {
        (0..self.screen_lines()).any(|l| {
            let row = &self.grid[Line(l as i32)];
            (0..self.columns()).any(|c| row[Column(c)].flags.contains(Flags::BLINK))
        })
    }

    /// Collect the information about the changes in the lines, which
    /// could be used to minimize the amount of drawing operations.
    ///
    /// The user controlled elements, like `Vi` mode cursor and `Selection` are **not** part of the
    /// collected damage state. Those could easily be tracked by comparing their old and new
    /// value between adjacent frames.
    ///
    /// After reading damage [`reset_damage`] should be called.
    ///
    /// [`reset_damage`]: Self::reset_damage
    #[must_use]
    pub fn damage(&mut self) -> TermDamage<'_> {
        // Ensure the entire terminal is damaged after entering insert mode.
        // Leaving is handled in the ansi handler.
        if self.mode.contains(TermMode::INSERT) {
            self.mark_fully_damaged();
        }

        let previous_cursor = mem::replace(&mut self.damage.last_cursor, self.grid.cursor.point);

        if self.damage.full {
            return TermDamage::Full;
        }

        // Add information about old cursor position and new one if they are not the same, so we
        // cover everything that was produced by `Term::input`.
        if self.damage.last_cursor != previous_cursor {
            // Cursor coordinates are always inside viewport even if you have `display_offset`.
            let point = Point::new(previous_cursor.line.0 as usize, previous_cursor.column);
            self.damage.damage_point(point);
        }

        // Always damage current cursor.
        self.damage_cursor();

        // NOTE: damage which changes all the content when the display offset is non-zero (e.g.
        // scrolling) is handled via full damage.
        let display_offset = self.grid().display_offset();
        TermDamage::Partial(TermDamageIterator::new(&self.damage.lines, display_offset))
    }

    /// Resets the terminal damage information.
    pub fn reset_damage(&mut self) {
        self.damage.reset(self.columns());
    }

    #[inline]
    fn mark_fully_damaged(&mut self) {
        self.damage.full = true;
    }

    /// Change only the OSC 52 mode (Rift: `set_options` would also resend
    /// the title and touch the scrollback).
    pub fn set_osc52(&mut self, osc52: Osc52) {
        self.config.osc52 = osc52;
    }

    /// Set new options for the [`Term`].
    pub fn set_options(&mut self, options: Config)
    where
        T: EventListener,
    {
        let old_config = mem::replace(&mut self.config, options);

        let title_event = match &self.title {
            Some(title) => Event::Title(title.clone()),
            None => Event::ResetTitle,
        };

        self.event_proxy.send_event(title_event);

        if self.mode.contains(TermMode::ALT_SCREEN) {
            self.inactive_grid.update_history(self.config.scrolling_history);
        } else {
            self.grid.update_history(self.config.scrolling_history);
        }

        if self.config.kitty_keyboard != old_config.kitty_keyboard {
            self.keyboard_mode_stack = Vec::new();
            self.inactive_keyboard_mode_stack = Vec::new();
            self.mode.remove(TermMode::KITTY_KEYBOARD_PROTOCOL);
        }

        // Damage everything on config updates.
        self.mark_fully_damaged();
    }

    /// Convert the active selection to a String.
    pub fn selection_to_string(&self) -> Option<String> {
        let selection_range = self.selection.as_ref().and_then(|s| s.to_range(self))?;
        let SelectionRange { start, end, .. } = selection_range;

        let mut res = String::new();

        match self.selection.as_ref() {
            Some(Selection { ty: SelectionType::Block, .. }) => {
                for line in (start.line.0..end.line.0).map(Line::from) {
                    res += self
                        .line_to_string(line, start.column..end.column, start.column.0 != 0)
                        .trim_end();
                    res += "\n";
                }

                res += self.line_to_string(end.line, start.column..end.column, true).trim_end();
            },
            Some(Selection { ty: SelectionType::Lines, .. }) => {
                res = self.bounds_to_string(start, end) + "\n";
            },
            _ => {
                res = self.bounds_to_string(start, end);
            },
        }

        Some(res)
    }

    /// Convert range between two points to a String.
    pub fn bounds_to_string(&self, start: Point, end: Point) -> String {
        let mut res = String::new();

        for line in (start.line.0..=end.line.0).map(Line::from) {
            let start_col = if line == start.line { start.column } else { Column(0) };
            let end_col = if line == end.line { end.column } else { self.last_column() };

            res += &self.line_to_string(line, start_col..end_col, line == end.line);
        }

        res.strip_suffix('\n').map(str::to_owned).unwrap_or(res)
    }

    /// Convert a single line in the grid to a String.
    fn line_to_string(
        &self,
        line: Line,
        mut cols: Range<Column>,
        include_wrapped_wide: bool,
    ) -> String {
        let mut text = String::new();

        let grid_line = &self.grid[line];
        let line_length = cmp::min(grid_line.line_length(), cols.end + 1);

        // Include wide char when trailing spacer is selected.
        if grid_line[cols.start].flags.contains(Flags::WIDE_CHAR_SPACER) {
            cols.start -= 1;
        }

        let mut tab_mode = false;
        for column in (cols.start.0..line_length.0).map(Column::from) {
            let cell = &grid_line[column];

            // Skip over cells until next tab-stop once a tab was found.
            if tab_mode {
                if self.tabs[column] || cell.c != ' ' {
                    tab_mode = false;
                } else {
                    continue;
                }
            }

            if cell.c == '\t' {
                tab_mode = true;
            }

            if !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                // Push cells primary character.
                text.push(cell.c);

                // Push zero-width characters.
                for c in cell.zerowidth().into_iter().flatten() {
                    text.push(*c);
                }
            }
        }

        if cols.end >= self.columns() - 1
            && (line_length.0 == 0
                || !self.grid[line][line_length - 1].flags.contains(Flags::WRAPLINE))
        {
            text.push('\n');
        }

        // If wide char is not part of the selection, but leading spacer is, include it.
        if line_length == self.columns()
            && line_length.0 >= 2
            && grid_line[line_length - 1].flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
            && include_wrapped_wide
        {
            text.push(self.grid[line - 1i32][Column(0)].c);
        }

        text
    }

    /// Terminal content required for rendering.
    #[inline]
    pub fn renderable_content(&self) -> RenderableContent<'_>
    where
        T: EventListener,
    {
        RenderableContent::new(self)
    }

    /// Access to the raw grid data structure.
    pub fn grid(&self) -> &Grid<Cell> {
        &self.grid
    }

    /// Mutable access to the raw grid data structure.
    pub fn grid_mut(&mut self) -> &mut Grid<Cell> {
        &mut self.grid
    }

    /// Resize terminal to new dimensions.
    pub fn resize<S: Dimensions>(&mut self, size: S) {
        let old_cols = self.columns();
        let old_lines = self.screen_lines();

        let num_cols = size.columns();
        let num_lines = size.screen_lines();

        if old_cols == num_cols && old_lines == num_lines {
            debug!("Term::resize dimensions unchanged");
            return;
        }

        debug!("New num_cols is {num_cols} and num_lines is {num_lines}");

        // Move vi mode cursor with the content.
        let history_size = self.history_size();
        let mut delta = num_lines as i32 - old_lines as i32;
        let min_delta = cmp::min(0, num_lines as i32 - self.grid.cursor.point.line.0 - 1);
        delta = cmp::min(cmp::max(delta, min_delta), history_size as i32);
        self.vi_mode_cursor.point.line += delta;

        let is_alt = self.mode.contains(TermMode::ALT_SCREEN);
        self.grid.resize(!is_alt, num_lines, num_cols);
        self.inactive_grid.resize(is_alt, num_lines, num_cols);

        // Invalidate selection and tabs only when necessary.
        if old_cols != num_cols {
            self.selection = None;

            // Recreate tabs list.
            self.tabs.resize(num_cols);
        } else if let Some(selection) = self.selection.take() {
            let max_lines = cmp::max(num_lines, old_lines) as i32;
            let range = Line(0)..Line(max_lines);
            self.selection = selection.rotate(self, &range, -delta);
        }

        // Clamp vi cursor to viewport.
        let vi_point = self.vi_mode_cursor.point;
        let viewport_top = Line(-(self.grid.display_offset() as i32));
        let viewport_bottom = viewport_top + self.bottommost_line();
        self.vi_mode_cursor.point.line =
            cmp::max(cmp::min(vi_point.line, viewport_bottom), viewport_top);
        self.vi_mode_cursor.point.column = cmp::min(vi_point.column, self.last_column());

        // Reset scrolling region.
        self.scroll_region = Line(0)..Line(self.screen_lines() as i32);
        self.vt.left = 0;
        self.vt.right = num_cols - 1;

        // Resize damage information.
        self.damage.resize(num_cols, num_lines);
    }

    /// Active terminal modes.
    #[inline]
    pub fn mode(&self) -> &TermMode {
        &self.mode
    }

    /// Swap primary and alternate screen buffer.
    pub fn swap_alt(&mut self) {
        if !self.mode.contains(TermMode::ALT_SCREEN) {
            // Set alt screen cursor to the current primary screen cursor.
            self.inactive_grid.cursor = self.grid.cursor.clone();

            // Drop information about the primary screens saved cursor.
            self.grid.saved_cursor = self.grid.cursor.clone();

            // Reset alternate screen contents.
            self.inactive_grid.reset_region(..);
        }

        mem::swap(&mut self.keyboard_mode_stack, &mut self.inactive_keyboard_mode_stack);
        let keyboard_mode =
            self.keyboard_mode_stack.last().copied().unwrap_or(KeyboardModes::NO_MODE).into();
        self.set_keyboard_mode(keyboard_mode, KeyboardModesApplyBehavior::Replace);

        mem::swap(&mut self.grid, &mut self.inactive_grid);
        self.mode ^= TermMode::ALT_SCREEN;
        self.selection = None;
        self.mark_fully_damaged();
    }

    /// Scroll screen down.
    ///
    /// Text moves down; clear at bottom
    /// Expects origin to be in scroll range.
    #[inline]
    fn scroll_down_relative(&mut self, origin: Line, mut lines: usize) {
        trace!("Scrolling down relative: origin={origin}, lines={lines}");

        lines = cmp::min(lines, (self.scroll_region.end - self.scroll_region.start).0 as usize);
        lines = cmp::min(lines, (self.scroll_region.end - origin).0 as usize);

        let region = origin..self.scroll_region.end;

        // Scroll selection.
        self.selection =
            self.selection.take().and_then(|s| s.rotate(self, &region, -(lines as i32)));

        // Scroll vi mode cursor.
        let line = &mut self.vi_mode_cursor.point.line;
        if region.start <= *line && region.end > *line {
            *line = cmp::min(*line + lines, region.end - 1);
        }

        // Scroll between origin and bottom
        self.grid.scroll_down(&region, lines);
        self.mark_fully_damaged();
    }

    /// Scroll screen up
    ///
    /// Text moves up; clear at top
    /// Expects origin to be in scroll range.
    #[inline]
    fn scroll_up_relative(&mut self, origin: Line, mut lines: usize) {
        trace!("Scrolling up relative: origin={origin}, lines={lines}");

        lines = cmp::min(lines, (self.scroll_region.end - self.scroll_region.start).0 as usize);

        let region = origin..self.scroll_region.end;

        // Scroll selection.
        self.selection = self.selection.take().and_then(|s| s.rotate(self, &region, lines as i32));

        self.grid.scroll_up(&region, lines);

        // Scroll vi mode cursor.
        let viewport_top = Line(-(self.grid.display_offset() as i32));
        let top = if region.start == 0 { viewport_top } else { region.start };
        let line = &mut self.vi_mode_cursor.point.line;
        if (top <= *line) && region.end > *line {
            *line = cmp::max(*line - lines, top);
        }
        self.mark_fully_damaged();
    }

    fn deccolm(&mut self, wide: bool)
    where
        T: EventListener,
    {
        // xterm ignores DECCOLM unless 80/132 switching is allowed (?40)
        if !self.vt.allow_column_switch {
            return;
        }
        self.vt.columns_132 = wide;
        // the window resizes to 80 or 132 columns
        let columns = if wide { 132 } else { 80 };
        self.request_resize(self.screen_lines(), columns);
        self.event_proxy.send_event(Event::WindowOp(vec![Some(8), None, Some(columns as u16)]));
        self.vt.declrmm = false;
        self.vt.left = 0;
        self.vt.right = self.columns() - 1;
        self.set_scrolling_region(1, None);
        // DECNCSM keeps the screen, but only on a VT510 (level 5)
        if !(self.vt.no_clear_on_column && self.vt.level >= 65) {
            self.grid.reset_region(..);
        }
        self.mark_fully_damaged();
    }

    #[inline]
    pub fn exit(&mut self)
    where
        T: EventListener,
    {
        self.event_proxy.send_event(Event::Exit);
    }

    /// Toggle the vi mode.
    #[inline]
    pub fn toggle_vi_mode(&mut self)
    where
        T: EventListener,
    {
        self.mode ^= TermMode::VI;

        if self.mode.contains(TermMode::VI) {
            let display_offset = self.grid.display_offset() as i32;
            if self.grid.cursor.point.line > self.bottommost_line() - display_offset {
                // Move cursor to top-left if terminal cursor is not visible.
                let point = Point::new(Line(-display_offset), Column(0));
                self.vi_mode_cursor = ViModeCursor::new(point);
            } else {
                // Reset vi mode cursor position to match primary cursor.
                self.vi_mode_cursor = ViModeCursor::new(self.grid.cursor.point);
            }
        }

        // Update UI about cursor blinking state changes.
        self.event_proxy.send_event(Event::CursorBlinkingChange);
    }

    /// Move vi mode cursor.
    #[inline]
    pub fn vi_motion(&mut self, motion: ViMotion)
    where
        T: EventListener,
    {
        // Require vi mode to be active.
        if !self.mode.contains(TermMode::VI) {
            return;
        }

        // Move cursor.
        self.vi_mode_cursor = self.vi_mode_cursor.motion(self, motion);
        self.vi_mode_recompute_selection();
    }

    /// Move vi cursor to a point in the grid.
    #[inline]
    pub fn vi_goto_point(&mut self, point: Point)
    where
        T: EventListener,
    {
        // Move viewport to make point visible.
        self.scroll_to_point(point);

        // Move vi cursor to the point.
        self.vi_mode_cursor.point = point;

        self.vi_mode_recompute_selection();
    }

    /// Update the active selection to match the vi mode cursor position.
    #[inline]
    fn vi_mode_recompute_selection(&mut self) {
        // Require vi mode to be active.
        if !self.mode.contains(TermMode::VI) {
            return;
        }

        // Update only if non-empty selection is present.
        if let Some(selection) = self.selection.as_mut().filter(|s| !s.is_empty()) {
            selection.update(self.vi_mode_cursor.point, Side::Left);
            selection.include_all();
        }
    }

    /// Scroll display to point if it is outside of viewport.
    pub fn scroll_to_point(&mut self, point: Point)
    where
        T: EventListener,
    {
        let display_offset = self.grid.display_offset() as i32;
        let screen_lines = self.grid.screen_lines() as i32;

        if point.line < -display_offset {
            let lines = point.line + display_offset;
            self.scroll_display(Scroll::Delta(-lines.0));
        } else if point.line >= (screen_lines - display_offset) {
            let lines = point.line + display_offset - screen_lines + 1i32;
            self.scroll_display(Scroll::Delta(-lines.0));
        }
    }

    /// Jump to the end of a wide cell.
    pub fn expand_wide(&self, mut point: Point, direction: Direction) -> Point {
        let flags = self.grid[point.line][point.column].flags;

        match direction {
            Direction::Right if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) => {
                point.column = Column(1);
                point.line += 1;
            },
            Direction::Right if flags.contains(Flags::WIDE_CHAR) => {
                point.column = cmp::min(point.column + 1, self.last_column());
            },
            Direction::Left if flags.intersects(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER) => {
                if flags.contains(Flags::WIDE_CHAR_SPACER) {
                    point.column -= 1;
                }

                let prev = point.sub(self, Boundary::Grid, 1);
                if self.grid[prev].flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                    point = prev;
                }
            },
            _ => (),
        }

        point
    }

    #[inline]
    pub fn semantic_escape_chars(&self) -> &str {
        &self.config.semantic_escape_chars
    }

    #[cfg(test)]
    pub(crate) fn set_semantic_escape_chars(&mut self, semantic_escape_chars: &str) {
        self.config.semantic_escape_chars = semantic_escape_chars.into();
    }

    /// Active terminal cursor style.
    ///
    /// While vi mode is active, this will automatically return the vi mode cursor style.
    #[inline]
    pub fn cursor_style(&self) -> CursorStyle {
        let cursor_style = self.cursor_style.unwrap_or(self.config.default_cursor_style);

        if self.mode.contains(TermMode::VI) {
            self.config.vi_mode_cursor_style.unwrap_or(cursor_style)
        } else {
            cursor_style
        }
    }

    pub fn colors(&self) -> &Colors {
        &self.colors
    }

    /// Insert a linebreak at the current cursor position.
    #[inline]
    fn wrapline(&mut self)
    where
        T: EventListener,
    {
        if !self.mode.contains(TermMode::LINE_WRAP) {
            return;
        }

        trace!("Wrapping input");

        self.grid.cursor_cell().flags.insert(Flags::WRAPLINE);

        if self.grid.cursor.point.line + 1 >= self.scroll_region.end {
            self.linefeed();
        } else {
            self.damage_cursor();
            self.grid.cursor.point.line += 1;
        }

        self.grid.cursor.point.column = Column(self.lr().0);
        self.grid.cursor.input_needs_wrap = false;
        self.damage_cursor();
    }

    /// Write `c` to the cell at the cursor position.
    #[inline(always)]
    fn write_at_cursor(&mut self, c: char) {
        let c = self.grid.cursor.charsets[self.active_charset].map(c);
        let fg = self.grid.cursor.template.fg;
        let bg = self.grid.cursor.template.bg;
        let flags = self.grid.cursor.template.flags;
        let extra = self.grid.cursor.template.extra.clone();

        let mut cursor_cell = self.grid.cursor_cell();

        // Clear all related cells when overwriting a fullwidth cell.
        if cursor_cell.flags.intersects(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER) {
            // Remove wide char and spacer.
            let wide = cursor_cell.flags.contains(Flags::WIDE_CHAR);
            let point = self.grid.cursor.point;
            if wide && point.column < self.last_column() {
                self.grid[point.line][point.column + 1].flags.remove(Flags::WIDE_CHAR_SPACER);
            } else if point.column > 0 {
                self.grid[point.line][point.column - 1].clear_wide();
            }

            // Remove leading spacers.
            if point.column <= 1 && point.line != self.topmost_line() {
                let column = self.last_column();
                self.grid[point.line - 1i32][column].flags.remove(Flags::LEADING_WIDE_CHAR_SPACER);
            }

            cursor_cell = self.grid.cursor_cell();
        }

        cursor_cell.c = c;
        cursor_cell.fg = fg;
        cursor_cell.bg = bg;
        cursor_cell.flags = flags;
        cursor_cell.extra = extra;
    }

    /// Cursor state a grapheme cluster continues from.
    fn cluster_cursor(&self) -> (i32, usize, bool) {
        let cursor = &self.grid.cursor;
        (cursor.point.line.0, cursor.point.column.0, cursor.input_needs_wrap)
    }

    /// Cells a line holds for text written at the cursor: up to the right
    /// margin when the cursor is inside it, else the screen edge; half on
    /// DECDWL/DECDHL lines.
    fn input_columns(&self) -> usize {
        let right = self.lr().1;
        let columns =
            if self.grid.cursor.point.column.0 <= right { right + 1 } else { self.columns() };
        if self.vt.line_sizes && self.grid[self.grid.cursor.point.line].attr != 0 {
            cmp::min(columns, self.columns() / 2)
        } else {
            columns
        }
    }

    /// The cell holding the cluster before the cursor.
    fn previous_cell(&self) -> Point {
        let mut column = self.grid.cursor.point.column;
        if !self.grid.cursor.input_needs_wrap {
            column.0 = column.saturating_sub(1);
        }
        // a wide character's characters live in its first cell
        let line = self.grid.cursor.point.line;
        if self.grid[line][column].flags.contains(Flags::WIDE_CHAR_SPACER) {
            column.0 = column.saturating_sub(1);
        }
        Point::new(line, column)
    }

    /// Adds `c` to the cluster before the cursor.
    fn join_previous(&mut self, c: char) {
        let point = self.previous_cell();
        self.grid[point].push_zerowidth(c);
    }

    /// Makes the one-cell cluster before the cursor two cells wide (VS16,
    /// conjuncts). At the end of the line it moves to the next one, as a
    /// wide character would have.
    fn widen_previous(&mut self)
    where
        T: EventListener,
    {
        let point = self.previous_cell();
        if self.grid[point].flags.intersects(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER) {
            return;
        }
        self.grid.cursor.point = point;
        self.grid.cursor.input_needs_wrap = false;
        let columns = self.input_columns();
        if point.column.0 + 1 >= columns {
            if !self.mode.contains(TermMode::LINE_WRAP) || columns < 2 {
                // no room and no wrapping: it stays one cell
                self.grid.cursor.input_needs_wrap = true;
                return;
            }
            let base = self.grid[point].clone();
            self.grid.cursor.template.flags.insert(Flags::LEADING_WIDE_CHAR_SPACER);
            self.write_at_cursor(' ');
            self.grid.cursor.template.flags.remove(Flags::LEADING_WIDE_CHAR_SPACER);
            self.wrapline();
            self.write_at_cursor(' ');
            *self.grid.cursor_cell() = base;
        }
        self.grid.cursor_cell().flags.insert(Flags::WIDE_CHAR);
        let base = self.grid.cursor.point;
        self.grid.cursor.point.column += 1;
        self.grid.cursor.template.flags.insert(Flags::WIDE_CHAR_SPACER);
        self.write_at_cursor(' ');
        self.grid.cursor.template.flags.remove(Flags::WIDE_CHAR_SPACER);
        // overwriting the old cell may have cleared the base's wide flag
        self.grid[base].flags.insert(Flags::WIDE_CHAR);
        if self.grid.cursor.point.column + 1 < self.input_columns() {
            self.grid.cursor.point.column += 1;
        } else {
            self.grid.cursor.input_needs_wrap = true;
        }
    }

    /// Makes the two-cell cluster before the cursor one cell wide (VS15).
    fn narrow_previous(&mut self) {
        let point = self.previous_cell();
        if !self.grid[point].flags.contains(Flags::WIDE_CHAR) {
            return;
        }
        self.grid[point].flags.remove(Flags::WIDE_CHAR);
        let spacer = Point::new(point.line, point.column + 1);
        if spacer.column.0 < self.columns() {
            let spacer = &mut self.grid[spacer];
            spacer.flags.remove(Flags::WIDE_CHAR_SPACER);
            spacer.c = ' ';
        }
        self.grid.cursor.point = spacer;
        self.grid.cursor.input_needs_wrap = false;
    }

    /// [`Handler::input`] past its ASCII fast path: joins, widens and narrows
    /// clusters (kept out of line so the fast path stays small).
    #[inline(never)]
    fn input_cluster(&mut self, c: char)
    where
        T: EventListener,
    {
        let here = self.cluster_cursor();
        if self.vt.cluster.cursor_after != Some(here) {
            self.vt.cluster.reset();
        }
        let Some(placement) = self.vt.cluster.place(c) else {
            return;
        };
        match placement {
            width::Placement::New(width) => self.put_char(c, width),
            width::Placement::Join => self.join_previous(c),
            width::Placement::JoinWiden => {
                self.join_previous(c);
                self.widen_previous();
            },
            width::Placement::JoinNarrow => {
                self.join_previous(c);
                self.narrow_previous();
            },
        }
        self.vt.cluster.cursor_after = Some(self.cluster_cursor());
    }

    /// Writes a character that starts a new cluster, `width` cells wide.
    #[inline(always)]
    fn put_char(&mut self, c: char, width: usize)
    where
        T: EventListener,
    {
        // Move cursor to next line.
        if self.grid.cursor.input_needs_wrap {
            self.wrapline();
        }

        // Text wraps at the right margin when the cursor is inside it, else
        // at the screen edge (`columns` is that edge + 1 below). DECDWL and
        // DECDHL lines hold half as many characters. Both checks sit behind
        // flags: this runs per character.
        let mut columns = self.columns();
        if self.vt.declrmm {
            let right = self.lr().1;
            if self.grid.cursor.point.column.0 <= right {
                columns = right + 1;
            }
        }
        if self.vt.line_sizes && self.grid[self.grid.cursor.point.line].attr != 0 {
            columns = cmp::min(columns, self.columns() / 2);
        }
        if self.grid.cursor.point.column.0 >= columns {
            self.grid.cursor.point.column = Column(columns - 1);
        }

        // If in insert mode, first shift cells to the right.
        if self.mode.contains(TermMode::INSERT) && self.grid.cursor.point.column + width < columns {
            let line = self.grid.cursor.point.line;
            let col = self.grid.cursor.point.column;
            let row = &mut self.grid[line][..];

            for col in (col.0..(columns - width)).rev() {
                row.swap(col + width, col);
            }
        }

        if width == 1 {
            self.write_at_cursor(c);
        } else {
            if self.grid.cursor.point.column + 1 >= columns {
                if self.mode.contains(TermMode::LINE_WRAP) {
                    // Insert placeholder before wide char if glyph does not fit in this row.
                    self.grid.cursor.template.flags.insert(Flags::LEADING_WIDE_CHAR_SPACER);
                    self.write_at_cursor(' ');
                    self.grid.cursor.template.flags.remove(Flags::LEADING_WIDE_CHAR_SPACER);
                    self.wrapline();
                } else {
                    // Prevent out of bounds crash when linewrapping is disabled.
                    self.grid.cursor.input_needs_wrap = true;
                    return;
                }
            }

            // Write full width glyph to current cursor cell.
            self.grid.cursor.template.flags.insert(Flags::WIDE_CHAR);
            self.write_at_cursor(c);
            self.grid.cursor.template.flags.remove(Flags::WIDE_CHAR);

            // Write spacer to cell following the wide glyph.
            self.grid.cursor.point.column += 1;
            self.grid.cursor.template.flags.insert(Flags::WIDE_CHAR_SPACER);
            self.write_at_cursor(' ');
            self.grid.cursor.template.flags.remove(Flags::WIDE_CHAR_SPACER);
        }

        if self.grid.cursor.point.column + 1 < columns {
            self.grid.cursor.point.column += 1;
        } else {
            self.grid.cursor.input_needs_wrap = true;
        }
    }
    #[inline]
    fn damage_cursor(&mut self) {
        // The normal cursor coordinates are always in viewport.
        let point =
            Point::new(self.grid.cursor.point.line.0 as usize, self.grid.cursor.point.column);
        self.damage.damage_point(point);
    }

    #[inline]
    fn set_keyboard_mode(&mut self, mode: TermMode, apply: KeyboardModesApplyBehavior) {
        let active_mode = self.mode & TermMode::KITTY_KEYBOARD_PROTOCOL;
        self.mode &= !TermMode::KITTY_KEYBOARD_PROTOCOL;
        let new_mode = match apply {
            KeyboardModesApplyBehavior::Replace => mode,
            KeyboardModesApplyBehavior::Union => active_mode.union(mode),
            KeyboardModesApplyBehavior::Difference => active_mode.difference(mode),
        };
        trace!("Setting keyboard mode to {new_mode:?}");
        self.mode |= new_mode;
    }
}

impl<T> Dimensions for Term<T> {
    #[inline]
    fn columns(&self) -> usize {
        self.grid.columns()
    }

    #[inline]
    fn screen_lines(&self) -> usize {
        self.grid.screen_lines()
    }

    #[inline]
    fn total_lines(&self) -> usize {
        self.grid.total_lines()
    }
}

impl<T: EventListener> Handler for Term<T> {
    /// A character to be displayed.
    #[inline(never)]
    fn input(&mut self, c: char) {
        // Grapheme clusters (emoji presentation, ZWJ sequences, flags,
        // conjuncts) as wcwidth measures them; a cursor motion since the
        // last character ends the cluster.
        if let Some(width) = self.vt.cluster.starts_cluster(c) {
            // the hot path: a visible character that joins nothing starts a
            // cluster whatever came before, so the motion check is not needed
            self.put_char(c, width);
            let after = self.cluster_cursor();
            self.vt.cluster.started(c, width, after);
            return;
        }
        self.input_cluster(c);
    }

    #[inline]
    fn decaln(&mut self) {
        trace!("Decalnning");

        for line in (0..self.screen_lines()).map(Line::from) {
            for column in 0..self.columns() {
                let cell = &mut self.grid[line][Column(column)];
                *cell = Cell::default();
                cell.c = 'E';
            }
        }
        self.scroll_region = Line(0)..Line(self.screen_lines() as i32);
        self.vt.left = 0;
        self.vt.right = self.columns() - 1;
        self.damage_cursor();
        self.grid.cursor.point = crate::index::Point::new(Line(0), Column(0));
        self.grid.cursor.input_needs_wrap = false;

        self.mark_fully_damaged();
    }

    #[inline]
    fn goto(&mut self, line: i32, col: usize) {
        let line = Line(line);
        let col = Column(col);

        trace!("Going to: line={line}, col={col}");
        let (y_offset, max_y, x_offset, max_x) = if self.mode.contains(TermMode::ORIGIN) {
            let (l, r) = self.lr();
            (self.scroll_region.start, self.scroll_region.end - 1, l, r)
        } else {
            (Line(0), self.bottommost_line(), 0, self.columns() - 1)
        };

        self.damage_cursor();
        self.grid.cursor.point.line = cmp::max(cmp::min(line + y_offset, max_y), Line(0));
        self.grid.cursor.point.column = cmp::min(col + Column(x_offset), Column(max_x));
        self.damage_cursor();
        self.grid.cursor.input_needs_wrap = false;
    }

    #[inline]
    fn goto_line(&mut self, line: i32) {
        trace!("Going to line: {line}");
        // VPA: the column stays where it is
        let (y_offset, max_y) = if self.mode.contains(TermMode::ORIGIN) {
            (self.scroll_region.start, self.scroll_region.end - 1)
        } else {
            (Line(0), self.bottommost_line())
        };
        self.damage_cursor();
        self.grid.cursor.point.line = cmp::max(cmp::min(Line(line) + y_offset, max_y), Line(0));
        self.damage_cursor();
        self.grid.cursor.input_needs_wrap = false;
    }

    #[inline]
    fn goto_col(&mut self, col: usize) {
        trace!("Going to column: {col}");
        // CHA/HPA: the line stays where it is
        let (x_offset, max_x) =
            if self.mode.contains(TermMode::ORIGIN) { self.lr() } else { (0, self.columns() - 1) };
        self.damage_cursor();
        self.grid.cursor.point.column = Column(cmp::min(col + x_offset, max_x));
        self.damage_cursor();
        self.grid.cursor.input_needs_wrap = false;
    }

    #[inline]
    fn insert_blank(&mut self, count: usize) {
        let (left, right) = self.lr();
        let col = self.grid.cursor.point.column.0;
        // outside the left/right margins ICH does nothing
        if col < left || col > right {
            return;
        }
        let columns = right + 1;
        self.grid.cursor.input_needs_wrap = false;
        let cursor = &self.grid.cursor;
        let bg = cursor.template.bg;

        // Ensure inserting within terminal bounds
        let count = cmp::min(count, columns - cursor.point.column.0);

        let source = cursor.point.column;
        let destination = cursor.point.column.0 + count;
        let num_cells = columns - destination;

        let line = cursor.point.line;
        self.damage.damage_line(line.0 as usize, 0, self.columns() - 1);

        let row = &mut self.grid[line][..];

        for offset in (0..num_cells).rev() {
            row.swap(destination + offset, source.0 + offset);
        }

        // Cells were just moved out toward the end of the line;
        // fill in between source and dest with blanks.
        for cell in &mut row[source.0..destination] {
            *cell = bg.into();
        }
    }

    #[inline]
    fn move_up(&mut self, lines: usize) {
        trace!("Moving up: {lines}");

        let (top, _) = self.tb();
        let line = self.grid.cursor.point.line.0 as usize;
        let stop = if line >= top { top } else { 0 };
        self.damage_cursor();
        self.grid.cursor.point.line = Line(cmp::max(line.saturating_sub(lines), stop) as i32);
        self.damage_cursor();
        self.grid.cursor.input_needs_wrap = false;
    }

    #[inline]
    fn move_down(&mut self, lines: usize) {
        trace!("Moving down: {lines}");

        let (_, bottom) = self.tb();
        let line = self.grid.cursor.point.line.0 as usize;
        let stop = if line <= bottom { bottom } else { self.screen_lines() - 1 };
        self.damage_cursor();
        self.grid.cursor.point.line = Line(cmp::min(line.saturating_add(lines), stop) as i32);
        self.damage_cursor();
        self.grid.cursor.input_needs_wrap = false;
    }

    #[inline]
    fn move_forward(&mut self, cols: usize) {
        trace!("Moving forward: {cols}");
        let right = self.lr().1;
        let stop = if self.grid.cursor.point.column.0 <= right {
            Column(right)
        } else {
            self.last_column()
        };
        let last_column = cmp::min(self.grid.cursor.point.column + cols, stop);

        let cursor_line = self.grid.cursor.point.line.0 as usize;
        self.damage.damage_line(cursor_line, self.grid.cursor.point.column.0, last_column.0);

        self.grid.cursor.point.column = last_column;
        self.grid.cursor.input_needs_wrap = false;
    }

    #[inline]
    fn move_backward(&mut self, cols: usize) {
        trace!("Moving backward: {cols}");
        self.cursor_back(cols);
    }

    #[inline]
    fn identify_terminal(&mut self, intermediate: Option<char>) {
        match intermediate {
            None => {
                trace!("Reporting primary device attributes");
                // the VT420 feature set (xterm's default build): 132 columns,
                // printer, selective erase, NRCS, technical set, locator,
                // state reports, user windows, horizontal scrolling, color,
                // rectangle editing, text locator
                let text = match self.vt.level {
                    61 => String::from("\x1b[?1;2c"),
                    62 => String::from("\x1b[?62;1;2;6;9;15;22;29c"),
                    63 => String::from("\x1b[?63;1;2;6;9;15;22;29c"),
                    level => format!("\x1b[?{level};1;2;6;9;15;16;17;18;21;22;28;29c"),
                };
                let _ = version_number;
                self.reply(text);
            },
            Some('>') => {
                trace!("Reporting secondary device attributes");
                // a VT420 (41); the xterm patch level it matches; no ROM
                self.reply(String::from("\x1b[>41;400;0c"));
            },
            Some('=') => {
                // DA3: unit id
                self.reply(String::from("\x1bP!|00000000\x1b\\"));
            },
            _ => debug!("Unsupported device attributes intermediate"),
        }
    }

    #[inline]
    fn report_keyboard_mode(&mut self) {
        if !self.config.kitty_keyboard {
            return;
        }

        trace!("Reporting active keyboard mode");
        // the active flags: `CSI = flags ; mode u` changes them in place
        let current_mode = KeyboardModes::from(self.mode).bits();
        let text = format!("\x1b[?{current_mode}u");
        self.event_proxy.send_event(Event::PtyWrite(text));
    }

    #[inline]
    fn push_keyboard_mode(&mut self, mode: KeyboardModes) {
        if !self.config.kitty_keyboard {
            return;
        }

        trace!("Pushing `{mode:?}` keyboard mode into the stack");

        if self.keyboard_mode_stack.len() >= KEYBOARD_MODE_STACK_MAX_DEPTH {
            let removed = self.keyboard_mode_stack.remove(0);
            trace!(
                "Removing '{removed:?}' from bottom of keyboard mode stack that exceeds its \
                 maximum depth"
            );
        }

        self.keyboard_mode_stack.push(mode);
        self.set_keyboard_mode(mode.into(), KeyboardModesApplyBehavior::Replace);
    }

    #[inline]
    fn pop_keyboard_modes(&mut self, to_pop: u16) {
        if !self.config.kitty_keyboard {
            return;
        }

        trace!("Attempting to pop {to_pop} keyboard modes from the stack");
        let new_len = self.keyboard_mode_stack.len().saturating_sub(to_pop as usize);
        self.keyboard_mode_stack.truncate(new_len);

        // Reload active mode.
        let mode = self.keyboard_mode_stack.last().copied().unwrap_or(KeyboardModes::NO_MODE);
        self.set_keyboard_mode(mode.into(), KeyboardModesApplyBehavior::Replace);
    }

    #[inline]
    fn set_keyboard_mode(&mut self, mode: KeyboardModes, apply: KeyboardModesApplyBehavior) {
        if !self.config.kitty_keyboard {
            return;
        }

        self.set_keyboard_mode(mode.into(), apply);
        // the flags at the top of the stack are the ones changed
        let active = KeyboardModes::from(self.mode);
        if let Some(top) = self.keyboard_mode_stack.last_mut() {
            *top = active;
        }
    }

    #[inline]
    fn device_status(&mut self, arg: usize) {
        trace!("Reporting device status: {arg}");
        match arg {
            5 => {
                let text = String::from("\x1b[0n");
                self.event_proxy.send_event(Event::PtyWrite(text));
            },
            6 => {
                let (row, col) = self.cursor_report();
                self.reply(format!("\x1b[{row};{col}R"));
            },
            _ => debug!("unknown device status query: {arg}"),
        };
    }

    #[inline]
    fn move_down_and_cr(&mut self, lines: usize) {
        trace!("Moving down and cr: {lines}");

        self.move_down(lines);
        self.carriage_return();
    }

    #[inline]
    fn move_up_and_cr(&mut self, lines: usize) {
        trace!("Moving up and cr: {lines}");

        self.move_up(lines);
        self.carriage_return();
    }

    /// Insert tab at cursor position.
    #[inline]
    fn put_tab(&mut self, mut count: u16) {
        // A tab in the pending-wrap position stays there (xterm), unless the
        // more(1) fix (?41) wraps first.
        if self.grid.cursor.input_needs_wrap {
            if !self.extended_private_mode(41).unwrap_or(false) {
                return;
            }
            self.wrapline();
        }

        let right = self.lr().1;
        let edge =
            if self.grid.cursor.point.column.0 <= right { right + 1 } else { self.columns() };
        while self.grid.cursor.point.column < edge && count != 0 {
            count -= 1;

            let c = self.grid.cursor.charsets[self.active_charset].map('\t');
            let cell = self.grid.cursor_cell();
            if cell.c == ' ' {
                cell.c = c;
            }

            loop {
                if (self.grid.cursor.point.column + 1) == edge {
                    break;
                }

                self.grid.cursor.point.column += 1;

                if self.tabs[self.grid.cursor.point.column] {
                    break;
                }
            }
        }
    }

    /// Backspace.
    #[inline]
    fn backspace(&mut self) {
        trace!("Backspace");

        self.cursor_back(1);
    }

    /// Carriage return.
    #[inline]
    fn carriage_return(&mut self) {
        trace!("Carriage return");
        let left = self.lr().0;
        let new_col = if self.grid.cursor.point.column.0 >= left { left } else { 0 };
        let line = self.grid.cursor.point.line.0 as usize;
        self.damage.damage_line(line, new_col, self.grid.cursor.point.column.0);
        self.grid.cursor.point.column = Column(new_col);
        self.grid.cursor.input_needs_wrap = false;
    }

    /// Linefeed.
    #[inline]
    fn linefeed(&mut self) {
        trace!("Linefeed");
        if self.mode.contains(TermMode::LINE_FEED_NEW_LINE) {
            self.carriage_return();
        }
        let next = self.grid.cursor.point.line + 1;
        if next == self.scroll_region.end {
            if self.cursor_in_lr() {
                self.scroll_up(1);
            }
        } else if next < self.screen_lines() {
            self.damage_cursor();
            self.grid.cursor.point.line += 1;
            self.damage_cursor();
        }
    }

    /// Set current position as a tabstop.
    #[inline]
    fn bell(&mut self) {
        trace!("Bell");
        self.event_proxy.send_event(Event::Bell);
    }

    #[inline]
    fn substitute(&mut self) {
        trace!("[unimplemented] Substitute");
    }

    /// Run LF/NL.
    ///
    /// LF/NL mode has some interesting history. According to ECMA-48 4th
    /// edition, in LINE FEED mode,
    ///
    /// > The execution of the formatter functions LINE FEED (LF), FORM FEED
    /// > (FF), LINE TABULATION (VT) cause only movement of the active position in
    /// > the direction of the line progression.
    ///
    /// In NEW LINE mode,
    ///
    /// > The execution of the formatter functions LINE FEED (LF), FORM FEED
    /// > (FF), LINE TABULATION (VT) cause movement to the line home position on
    /// > the following line, the following form, etc. In the case of LF this is
    /// > referred to as the New Line (NL) option.
    ///
    /// Additionally, ECMA-48 4th edition says that this option is deprecated.
    /// ECMA-48 5th edition only mentions this option (without explanation)
    /// saying that it's been removed.
    ///
    /// As an emulator, we need to support it since applications may still rely
    /// on it.
    #[inline]
    fn newline(&mut self) {
        self.linefeed();

        if self.mode.contains(TermMode::LINE_FEED_NEW_LINE) {
            self.carriage_return();
        }
    }

    #[inline]
    fn set_horizontal_tabstop(&mut self) {
        trace!("Setting horizontal tabstop");
        self.tabs[self.grid.cursor.point.column] = true;
    }

    #[inline]
    fn scroll_up(&mut self, lines: usize) {
        if self.has_lr_margins() {
            let (top, bottom) = self.tb();
            let (left, right) = self.lr();
            let area = vt420::Area { top, left, bottom, right };
            self.shift_area(area, -(lines as isize), 0);
            return;
        }
        let origin = self.scroll_region.start;
        self.scroll_up_relative(origin, lines);
    }

    #[inline]
    fn scroll_down(&mut self, lines: usize) {
        if self.has_lr_margins() {
            let (top, bottom) = self.tb();
            let (left, right) = self.lr();
            let area = vt420::Area { top, left, bottom, right };
            self.shift_area(area, lines as isize, 0);
            return;
        }
        let origin = self.scroll_region.start;
        self.scroll_down_relative(origin, lines);
    }

    #[inline]
    fn insert_blank_lines(&mut self, lines: usize) {
        trace!("Inserting blank {lines} lines");

        if !self.cursor_in_tb() || !self.cursor_in_lr() {
            return;
        }
        let origin = self.grid.cursor.point.line;
        if self.has_lr_margins() {
            let (_, bottom) = self.tb();
            let (left, right) = self.lr();
            let area = vt420::Area { top: origin.0 as usize, left, bottom, right };
            self.shift_area(area, lines as isize, 0);
        } else {
            self.scroll_down_relative(origin, lines);
        }
        self.grid.cursor.point.column = Column(self.lr().0);
        self.grid.cursor.input_needs_wrap = false;
    }

    #[inline]
    fn delete_lines(&mut self, lines: usize) {
        let origin = self.grid.cursor.point.line;
        let lines = cmp::min(self.screen_lines() - origin.0 as usize, lines);

        trace!("Deleting {lines} lines");

        if lines == 0 || !self.cursor_in_tb() || !self.cursor_in_lr() {
            return;
        }
        if self.has_lr_margins() {
            let (_, bottom) = self.tb();
            let (left, right) = self.lr();
            let area = vt420::Area { top: origin.0 as usize, left, bottom, right };
            self.shift_area(area, -(lines as isize), 0);
        } else {
            self.scroll_up_relative(origin, lines);
        }
        self.grid.cursor.point.column = Column(self.lr().0);
        self.grid.cursor.input_needs_wrap = false;
    }

    #[inline]
    fn erase_chars(&mut self, count: usize) {
        let cursor = &self.grid.cursor;

        trace!("Erasing chars: count={}, col={}", count, cursor.point.column);

        let start = cursor.point.column;
        let end = cmp::min(start + count, Column(self.columns()));

        // Cleared cells have current background color set.
        let bg = self.grid.cursor.template.bg;
        let line = cursor.point.line;
        self.damage.damage_line(line.0 as usize, start.0, end.0);
        // SPA/EPA-protected cells survive in ISO protection mode (DECSCA
        // protection never does)
        let keep = if self.vt.iso_mode { Flags::ISO_PROTECTED } else { Flags::empty() };
        let row = &mut self.grid[line];
        for cell in &mut row[start..end] {
            if !cell.flags.intersects(keep) {
                *cell = bg.into();
            }
        }
    }

    #[inline]
    fn delete_chars(&mut self, count: usize) {
        let (left, right) = self.lr();
        let col = self.grid.cursor.point.column.0;
        if col < left || col > right {
            return;
        }
        self.grid.cursor.input_needs_wrap = false;
        let columns = right + 1;
        let cursor = &self.grid.cursor;
        let bg = cursor.template.bg;

        let start = cursor.point.column.0;
        // Ensure deleting within the margin.
        let count = cmp::min(count, columns - start);

        let end = start + count;
        let num_cells = columns - end;

        let line = cursor.point.line;
        self.damage.damage_line(line.0 as usize, 0, self.columns() - 1);
        let row = &mut self.grid[line][..];

        for offset in 0..num_cells {
            row.swap(start + offset, end + offset);
        }

        // Clear the last `count` cells inside the margin.
        for cell in &mut row[columns - count..columns] {
            *cell = bg.into();
        }
    }

    #[inline]
    fn move_backward_tabs(&mut self, count: u16) {
        trace!("Moving backward {count} tabs");

        let old_col = self.grid.cursor.point.column.0;
        for _ in 0..count {
            let mut col = self.grid.cursor.point.column;

            if col.0 == 0 {
                break;
            }

            col = index::Column(0);
            for i in (0..(self.grid.cursor.point.column.0)).rev() {
                if self.tabs[index::Column(i)] {
                    col = index::Column(i);
                    break;
                }
            }
            self.grid.cursor.point.column = col;
        }

        let line = self.grid.cursor.point.line.0 as usize;
        self.damage.damage_line(line, self.grid.cursor.point.column.0, old_col);
    }

    #[inline]
    fn move_forward_tabs(&mut self, count: u16) {
        trace!("Moving forward {count} tabs");

        let right = self.lr().1;
        let num_cols =
            if self.grid.cursor.point.column.0 <= right { right + 1 } else { self.columns() };
        let old_col = self.grid.cursor.point.column.0;
        self.grid.cursor.input_needs_wrap = false;
        for _ in 0..count {
            let mut col = self.grid.cursor.point.column;

            if col == num_cols - 1 {
                break;
            }

            for i in col.0 + 1..num_cols {
                col = index::Column(i);
                if self.tabs[col] {
                    break;
                }
            }

            self.grid.cursor.point.column = col;
        }

        let line = self.grid.cursor.point.line.0 as usize;
        self.damage.damage_line(line, old_col, self.grid.cursor.point.column.0);
    }

    #[inline]
    fn save_cursor_position(&mut self) {
        trace!("Saving cursor position");

        self.grid.saved_cursor = self.grid.cursor.clone();
        self.grid.saved_cursor.origin = self.mode.contains(TermMode::ORIGIN);
    }

    #[inline]
    fn restore_cursor_position(&mut self) {
        trace!("Restoring cursor position");

        self.damage_cursor();
        self.grid.cursor = self.grid.saved_cursor.clone();
        self.mode.set(TermMode::ORIGIN, self.grid.saved_cursor.origin);
        // the saved position may lie past a since-narrowed screen
        let (lines, columns) = (self.grid.screen_lines(), self.grid.columns());
        let point = &mut self.grid.cursor.point;
        point.line = cmp::min(point.line, Line(lines as i32 - 1));
        point.column = cmp::min(point.column, Column(columns - 1));
        self.damage_cursor();
    }

    #[inline]
    fn clear_line(&mut self, mode: ansi::LineClearMode) {
        trace!("Clearing line: {mode:?}");

        let cursor = &self.grid.cursor;
        let bg = cursor.template.bg;
        let point = cursor.point;

        let (left, right) = match mode {
            ansi::LineClearMode::Right if cursor.input_needs_wrap => return,
            ansi::LineClearMode::Right => (point.column, Column(self.columns())),
            ansi::LineClearMode::Left => (Column(0), point.column + 1),
            ansi::LineClearMode::All => (Column(0), Column(self.columns())),
        };

        self.damage.damage_line(point.line.0 as usize, left.0, right.0 - 1);

        let keep = if self.vt.iso_mode { Flags::ISO_PROTECTED } else { Flags::empty() };
        let row = &mut self.grid[point.line];
        for cell in &mut row[left..right] {
            if !cell.flags.intersects(keep) {
                *cell = bg.into();
            }
        }

        let range = self.grid.cursor.point.line..=self.grid.cursor.point.line;
        self.selection = self.selection.take().filter(|s| !s.intersects_range(range));
    }

    /// Set the indexed color value.
    #[inline]
    fn set_color(&mut self, index: usize, color: Rgb) {
        trace!("Setting color[{index}] = {color:?}");

        // Damage terminal if the color changed and it's not the cursor.
        if index != NamedColor::Cursor as usize && self.colors[index] != Some(color) {
            self.mark_fully_damaged();
        }

        self.colors[index] = Some(color);
    }

    /// Respond to a color query escape sequence.
    #[inline]
    fn dynamic_color_sequence(&mut self, prefix: String, index: usize, terminator: &str) {
        trace!("Requested write of escape sequence for color code {prefix}: color[{index}]");

        let terminator = terminator.to_owned();
        self.event_proxy.send_event(Event::ColorRequest(
            index,
            Arc::new(move |color| {
                format!(
                    "\x1b]{};rgb:{1:02x}{1:02x}/{2:02x}{2:02x}/{3:02x}{3:02x}{4}",
                    prefix, color.r, color.g, color.b, terminator
                )
            }),
        ));
    }

    /// Reset the indexed color to original value.
    #[inline]
    fn reset_color(&mut self, index: usize) {
        trace!("Resetting color[{index}]");

        // Damage terminal if the color changed and it's not the cursor.
        if index != NamedColor::Cursor as usize && self.colors[index].is_some() {
            self.mark_fully_damaged();
        }

        self.colors[index] = None;
    }

    /// Store data into clipboard.
    #[inline]
    fn clipboard_store(&mut self, clipboard: u8, base64: &[u8]) {
        if !matches!(self.config.osc52, Osc52::OnlyCopy | Osc52::CopyPaste) {
            debug!("Denied osc52 store");
            return;
        }

        let clipboard_type = match clipboard {
            b'c' => ClipboardType::Clipboard,
            b'p' | b's' => ClipboardType::Selection,
            _ => return,
        };

        if let Ok(bytes) = Base64.decode(base64) {
            if let Ok(text) = String::from_utf8(bytes) {
                self.event_proxy.send_event(Event::ClipboardStore(clipboard_type, text));
            }
        }
    }

    /// Load data from clipboard.
    #[inline]
    fn clipboard_load(&mut self, clipboard: u8, terminator: &str) {
        if !matches!(self.config.osc52, Osc52::OnlyPaste | Osc52::CopyPaste) {
            debug!("Denied osc52 load");
            return;
        }

        let clipboard_type = match clipboard {
            b'c' => ClipboardType::Clipboard,
            b'p' | b's' => ClipboardType::Selection,
            _ => return,
        };

        let terminator = terminator.to_owned();

        self.event_proxy.send_event(Event::ClipboardLoad(
            clipboard_type,
            Arc::new(move |text| {
                let base64 = Base64.encode(text);
                format!("\x1b]52;{};{}{}", clipboard as char, base64, terminator)
            }),
        ));
    }

    #[inline]
    fn clear_screen(&mut self, mode: ansi::ClearMode) {
        trace!("Clearing screen: {mode:?}");
        let bg = self.grid.cursor.template.bg;

        let screen_lines = self.screen_lines();

        // SPA/EPA-protected text survives ED: erase cell by cell around it
        if !matches!(mode, ansi::ClearMode::Saved)
            && self.vt.iso_mode
            && self.screen_has(Flags::ISO_PROTECTED)
        {
            let point = self.grid.cursor.point;
            let (line, col) = (point.line.0 as usize, point.column.0);
            let last_col = self.columns() - 1;
            let last_line = screen_lines - 1;
            let keep = Flags::ISO_PROTECTED;
            let area = |top, left, bottom, right| vt420::Area { top, left, bottom, right };
            match mode {
                ansi::ClearMode::Below => {
                    self.erase_area_keeping(area(line, col, line, last_col), false, keep);
                    if line < last_line {
                        self.erase_area_keeping(
                            area(line + 1, 0, last_line, last_col),
                            false,
                            keep,
                        );
                    }
                },
                ansi::ClearMode::Above => {
                    if line > 0 {
                        self.erase_area_keeping(area(0, 0, line - 1, last_col), false, keep);
                    }
                    self.erase_area_keeping(area(line, 0, line, col), false, keep);
                },
                _ => self.erase_area_keeping(area(0, 0, last_line, last_col), false, keep),
            }
            return;
        }

        match mode {
            ansi::ClearMode::Above => {
                let cursor = self.grid.cursor.point;

                // Fully clear the lines above the current one (Rift: upstream
                // tested `> 1` and kept line 0 when the cursor was on line 1).
                if cursor.line > 0 {
                    // Fully clear all lines before the current line.
                    self.grid.reset_region(..cursor.line);
                }

                // Clear up to the current column in the current line.
                let end = cmp::min(cursor.column + 1, Column(self.columns()));
                for cell in &mut self.grid[cursor.line][..end] {
                    *cell = bg.into();
                }

                let range = Line(0)..=cursor.line;
                self.selection = self.selection.take().filter(|s| !s.intersects_range(range));
            },
            ansi::ClearMode::Below => {
                let cursor = self.grid.cursor.point;
                for cell in &mut self.grid[cursor.line][cursor.column..] {
                    *cell = bg.into();
                }

                if (cursor.line.0 as usize) < screen_lines - 1 {
                    self.grid.reset_region((cursor.line + 1)..);
                }

                let range = cursor.line..Line(screen_lines as i32);
                self.selection = self.selection.take().filter(|s| !s.intersects_range(range));
            },
            ansi::ClearMode::All => {
                if self.mode.contains(TermMode::ALT_SCREEN) {
                    self.grid.reset_region(..);
                } else {
                    let old_offset = self.grid.display_offset();

                    self.grid.clear_viewport();

                    // Compute number of lines scrolled by clearing the viewport.
                    let lines = self.grid.display_offset().saturating_sub(old_offset);

                    self.vi_mode_cursor.point.line =
                        (self.vi_mode_cursor.point.line - lines).grid_clamp(self, Boundary::Grid);
                }

                self.selection = None;
            },
            ansi::ClearMode::Saved if self.history_size() > 0 => {
                self.grid.clear_history();

                self.vi_mode_cursor.point.line =
                    self.vi_mode_cursor.point.line.grid_clamp(self, Boundary::Cursor);

                self.selection = self.selection.take().filter(|s| !s.intersects_range(..Line(0)));
            },
            // We have no history to clear.
            ansi::ClearMode::Saved => (),
        }

        self.mark_fully_damaged();
    }

    #[inline]
    fn clear_tabs(&mut self, mode: ansi::TabulationClearMode) {
        trace!("Clearing tabs: {mode:?}");
        match mode {
            ansi::TabulationClearMode::Current => {
                self.tabs[self.grid.cursor.point.column] = false;
            },
            ansi::TabulationClearMode::All => {
                self.tabs.clear_all();
            },
        }
    }

    /// Reset all important fields in the term struct.
    #[inline]
    fn reset_state(&mut self) {
        if self.mode.contains(TermMode::ALT_SCREEN) {
            mem::swap(&mut self.grid, &mut self.inactive_grid);
        }
        self.active_charset = Default::default();
        self.cursor_style = None;
        self.grid.reset();
        self.inactive_grid.reset();
        self.scroll_region = Line(0)..Line(self.screen_lines() as i32);
        let was_132 = self.vt.columns_132;
        self.vt = vt420::Vt420::new(self.columns());
        if was_132 {
            // RIS returns a DECCOLM screen to 80 columns
            self.request_resize(self.screen_lines(), 80);
            self.event_proxy.send_event(Event::WindowOp(vec![Some(8), None, Some(80)]));
        }
        self.colors = Colors::default();
        self.tabs = TabStops::new(self.columns());
        self.title_stack = Vec::new();
        self.title = None;
        self.selection = None;
        self.vi_mode_cursor = Default::default();
        self.keyboard_mode_stack = Default::default();
        self.inactive_keyboard_mode_stack = Default::default();

        // Preserve vi mode across resets.
        self.mode &= TermMode::VI;
        self.mode.insert(TermMode::default());

        self.event_proxy.send_event(Event::CursorBlinkingChange);
        self.mark_fully_damaged();
    }

    #[inline]
    fn reverse_index(&mut self) {
        trace!("Reversing index");
        // If cursor is at the top.
        if self.grid.cursor.point.line == self.scroll_region.start {
            if self.cursor_in_lr() {
                self.scroll_down(1);
            }
        } else {
            self.damage_cursor();
            self.grid.cursor.point.line = cmp::max(self.grid.cursor.point.line - 1, Line(0));
            self.damage_cursor();
        }
    }

    #[inline]
    fn set_hyperlink(&mut self, hyperlink: Option<Hyperlink>) {
        trace!("Setting hyperlink: {hyperlink:?}");
        self.grid.cursor.template.set_hyperlink(hyperlink.map(|e| e.into()));
    }

    /// Set a terminal attribute.
    #[inline]
    fn terminal_attribute(&mut self, attr: Attr) {
        trace!("Setting attribute: {attr:?}");
        let cursor = &mut self.grid.cursor;
        match attr {
            Attr::Foreground(color) => cursor.template.fg = color,
            Attr::Background(color) => cursor.template.bg = color,
            Attr::UnderlineColor(color) => cursor.template.set_underline_color(color),
            Attr::Reset => {
                cursor.template.fg = Color::Named(NamedColor::Foreground);
                cursor.template.bg = Color::Named(NamedColor::Background);
                // SGR 0 leaves the DECSCA protection alone
                cursor.template.flags &= Flags::PROTECTED;
                cursor.template.set_underline_color(None);
            },
            Attr::Reverse => cursor.template.flags.insert(Flags::INVERSE),
            Attr::CancelReverse => cursor.template.flags.remove(Flags::INVERSE),
            Attr::Bold => cursor.template.flags.insert(Flags::BOLD),
            Attr::CancelBold => cursor.template.flags.remove(Flags::BOLD),
            Attr::Dim => cursor.template.flags.insert(Flags::DIM),
            Attr::CancelBoldDim => cursor.template.flags.remove(Flags::BOLD | Flags::DIM),
            Attr::Italic => cursor.template.flags.insert(Flags::ITALIC),
            Attr::CancelItalic => cursor.template.flags.remove(Flags::ITALIC),
            Attr::Underline => {
                cursor.template.flags.remove(Flags::ALL_UNDERLINES);
                cursor.template.flags.insert(Flags::UNDERLINE);
            },
            Attr::DoubleUnderline => {
                cursor.template.flags.remove(Flags::ALL_UNDERLINES);
                cursor.template.flags.insert(Flags::DOUBLE_UNDERLINE);
            },
            Attr::Undercurl => {
                cursor.template.flags.remove(Flags::ALL_UNDERLINES);
                cursor.template.flags.insert(Flags::UNDERCURL);
            },
            Attr::DottedUnderline => {
                cursor.template.flags.remove(Flags::ALL_UNDERLINES);
                cursor.template.flags.insert(Flags::DOTTED_UNDERLINE);
            },
            Attr::DashedUnderline => {
                cursor.template.flags.remove(Flags::ALL_UNDERLINES);
                cursor.template.flags.insert(Flags::DASHED_UNDERLINE);
            },
            Attr::CancelUnderline => cursor.template.flags.remove(Flags::ALL_UNDERLINES),
            Attr::Hidden => cursor.template.flags.insert(Flags::HIDDEN),
            Attr::CancelHidden => cursor.template.flags.remove(Flags::HIDDEN),
            Attr::Strike => cursor.template.flags.insert(Flags::STRIKEOUT),
            Attr::CancelStrike => cursor.template.flags.remove(Flags::STRIKEOUT),
            Attr::BlinkSlow | Attr::BlinkFast => cursor.template.flags.insert(Flags::BLINK),
            Attr::CancelBlink => cursor.template.flags.remove(Flags::BLINK),
        }
    }

    #[inline]
    fn set_private_mode(&mut self, mode: PrivateMode) {
        let mode = match mode {
            PrivateMode::Named(mode) => mode,
            PrivateMode::Unknown(mode) => {
                self.set_extended_private_mode(mode, true);
                return;
            },
        };

        trace!("Setting private mode: {mode:?}");
        match mode {
            NamedPrivateMode::UrgencyHints => self.mode.insert(TermMode::URGENCY_HINTS),
            NamedPrivateMode::SwapScreenAndSetRestoreCursor => {
                if !self.mode.contains(TermMode::ALT_SCREEN) {
                    self.swap_alt();
                }
            },
            NamedPrivateMode::ShowCursor => self.mode.insert(TermMode::SHOW_CURSOR),
            NamedPrivateMode::CursorKeys => self.mode.insert(TermMode::APP_CURSOR),
            // Mouse protocols are mutually exclusive.
            NamedPrivateMode::ReportMouseClicks => {
                self.mode.remove(TermMode::MOUSE_MODE);
                self.mode.insert(TermMode::MOUSE_REPORT_CLICK);
                self.event_proxy.send_event(Event::MouseCursorDirty);
            },
            NamedPrivateMode::ReportCellMouseMotion => {
                self.mode.remove(TermMode::MOUSE_MODE);
                self.mode.insert(TermMode::MOUSE_DRAG);
                self.event_proxy.send_event(Event::MouseCursorDirty);
            },
            NamedPrivateMode::ReportAllMouseMotion => {
                self.mode.remove(TermMode::MOUSE_MODE);
                self.mode.insert(TermMode::MOUSE_MOTION);
                self.event_proxy.send_event(Event::MouseCursorDirty);
            },
            NamedPrivateMode::ReportFocusInOut => self.mode.insert(TermMode::FOCUS_IN_OUT),
            NamedPrivateMode::BracketedPaste => self.mode.insert(TermMode::BRACKETED_PASTE),
            // Mouse encodings are mutually exclusive.
            NamedPrivateMode::SgrMouse => {
                self.mode.remove(TermMode::UTF8_MOUSE);
                self.mode.insert(TermMode::SGR_MOUSE);
            },
            NamedPrivateMode::Utf8Mouse => {
                self.mode.remove(TermMode::SGR_MOUSE);
                self.mode.insert(TermMode::UTF8_MOUSE);
            },
            NamedPrivateMode::AlternateScroll => self.mode.insert(TermMode::ALTERNATE_SCROLL),
            NamedPrivateMode::LineWrap => self.mode.insert(TermMode::LINE_WRAP),
            NamedPrivateMode::Origin => {
                self.mode.insert(TermMode::ORIGIN);
                self.goto(0, 0);
            },
            NamedPrivateMode::ColumnMode => self.deccolm(true),
            NamedPrivateMode::BlinkingCursor => {
                let style = self.cursor_style.get_or_insert(self.config.default_cursor_style);
                style.blinking = true;
                self.event_proxy.send_event(Event::CursorBlinkingChange);
            },
            NamedPrivateMode::SyncUpdate => (),
        }
    }

    #[inline]
    fn unset_private_mode(&mut self, mode: PrivateMode) {
        let mode = match mode {
            PrivateMode::Named(mode) => mode,
            PrivateMode::Unknown(mode) => {
                self.set_extended_private_mode(mode, false);
                return;
            },
        };

        trace!("Unsetting private mode: {mode:?}");
        match mode {
            NamedPrivateMode::UrgencyHints => self.mode.remove(TermMode::URGENCY_HINTS),
            NamedPrivateMode::SwapScreenAndSetRestoreCursor => {
                if self.mode.contains(TermMode::ALT_SCREEN) {
                    self.swap_alt();
                }
            },
            NamedPrivateMode::ShowCursor => self.mode.remove(TermMode::SHOW_CURSOR),
            NamedPrivateMode::CursorKeys => self.mode.remove(TermMode::APP_CURSOR),
            NamedPrivateMode::ReportMouseClicks => {
                self.mode.remove(TermMode::MOUSE_REPORT_CLICK);
                self.event_proxy.send_event(Event::MouseCursorDirty);
            },
            NamedPrivateMode::ReportCellMouseMotion => {
                self.mode.remove(TermMode::MOUSE_DRAG);
                self.event_proxy.send_event(Event::MouseCursorDirty);
            },
            NamedPrivateMode::ReportAllMouseMotion => {
                self.mode.remove(TermMode::MOUSE_MOTION);
                self.event_proxy.send_event(Event::MouseCursorDirty);
            },
            NamedPrivateMode::ReportFocusInOut => self.mode.remove(TermMode::FOCUS_IN_OUT),
            NamedPrivateMode::BracketedPaste => self.mode.remove(TermMode::BRACKETED_PASTE),
            NamedPrivateMode::SgrMouse => self.mode.remove(TermMode::SGR_MOUSE),
            NamedPrivateMode::Utf8Mouse => self.mode.remove(TermMode::UTF8_MOUSE),
            NamedPrivateMode::AlternateScroll => self.mode.remove(TermMode::ALTERNATE_SCROLL),
            NamedPrivateMode::LineWrap => self.mode.remove(TermMode::LINE_WRAP),
            NamedPrivateMode::Origin => self.mode.remove(TermMode::ORIGIN),
            NamedPrivateMode::ColumnMode => self.deccolm(false),
            NamedPrivateMode::BlinkingCursor => {
                let style = self.cursor_style.get_or_insert(self.config.default_cursor_style);
                style.blinking = false;
                self.event_proxy.send_event(Event::CursorBlinkingChange);
            },
            NamedPrivateMode::SyncUpdate => (),
        }
    }

    #[inline]
    fn report_private_mode(&mut self, mode: PrivateMode) {
        trace!("Reporting private mode {mode:?}");
        if self.vt.level < 63 {
            return;
        }
        // DECARM (auto-repeat) and DECHCCM are fixed in xterm
        if matches!(mode.raw(), 8 | 60) {
            self.report_mode_state(true, mode.raw(), 4);
            return;
        }
        let state = match mode {
            PrivateMode::Named(mode) => match mode {
                NamedPrivateMode::CursorKeys => self.mode.contains(TermMode::APP_CURSOR).into(),
                NamedPrivateMode::Origin => self.mode.contains(TermMode::ORIGIN).into(),
                NamedPrivateMode::LineWrap => self.mode.contains(TermMode::LINE_WRAP).into(),
                NamedPrivateMode::BlinkingCursor => {
                    let style = self.cursor_style.get_or_insert(self.config.default_cursor_style);
                    style.blinking.into()
                },
                NamedPrivateMode::ShowCursor => self.mode.contains(TermMode::SHOW_CURSOR).into(),
                NamedPrivateMode::ReportMouseClicks => {
                    self.mode.contains(TermMode::MOUSE_REPORT_CLICK).into()
                },
                NamedPrivateMode::ReportCellMouseMotion => {
                    self.mode.contains(TermMode::MOUSE_DRAG).into()
                },
                NamedPrivateMode::ReportAllMouseMotion => {
                    self.mode.contains(TermMode::MOUSE_MOTION).into()
                },
                NamedPrivateMode::ReportFocusInOut => {
                    self.mode.contains(TermMode::FOCUS_IN_OUT).into()
                },
                NamedPrivateMode::Utf8Mouse => self.mode.contains(TermMode::UTF8_MOUSE).into(),
                NamedPrivateMode::SgrMouse => self.mode.contains(TermMode::SGR_MOUSE).into(),
                NamedPrivateMode::AlternateScroll => {
                    self.mode.contains(TermMode::ALTERNATE_SCROLL).into()
                },
                NamedPrivateMode::UrgencyHints => {
                    self.mode.contains(TermMode::URGENCY_HINTS).into()
                },
                NamedPrivateMode::SwapScreenAndSetRestoreCursor => {
                    self.mode.contains(TermMode::ALT_SCREEN).into()
                },
                NamedPrivateMode::BracketedPaste => {
                    self.mode.contains(TermMode::BRACKETED_PASTE).into()
                },
                NamedPrivateMode::SyncUpdate => ModeState::Reset,
                NamedPrivateMode::ColumnMode => self.vt.columns_132.into(),
            },
            PrivateMode::Unknown(n) => match self.extended_private_mode(n) {
                Some(on) => on.into(),
                None => ModeState::NotSupported,
            },
        };

        self.report_mode_state(true, mode.raw(), state as u8);
    }

    #[inline]
    fn set_mode(&mut self, mode: ansi::Mode) {
        let mode = match mode {
            ansi::Mode::Named(mode) => mode,
            ansi::Mode::Unknown(mode) => {
                self.set_extended_ansi_mode(mode, true);
                return;
            },
        };

        trace!("Setting public mode: {mode:?}");
        match mode {
            NamedMode::Insert => self.mode.insert(TermMode::INSERT),
            NamedMode::LineFeedNewLine => self.mode.insert(TermMode::LINE_FEED_NEW_LINE),
        }
    }

    #[inline]
    fn unset_mode(&mut self, mode: ansi::Mode) {
        let mode = match mode {
            ansi::Mode::Named(mode) => mode,
            ansi::Mode::Unknown(mode) => {
                self.set_extended_ansi_mode(mode, false);
                return;
            },
        };

        trace!("Setting public mode: {mode:?}");
        match mode {
            NamedMode::Insert => {
                self.mode.remove(TermMode::INSERT);
                self.mark_fully_damaged();
            },
            NamedMode::LineFeedNewLine => self.mode.remove(TermMode::LINE_FEED_NEW_LINE),
        }
    }

    #[inline]
    fn report_mode(&mut self, mode: ansi::Mode) {
        trace!("Reporting mode {mode:?}");
        if self.vt.level < 63 {
            return;
        }
        // ECMA-48 modes xterm keeps permanently reset
        if [1, 5, 7, 10, 11, 13, 14, 15, 16, 17, 18, 19].contains(&mode.raw()) {
            self.report_mode_state(false, mode.raw(), 4);
            return;
        }
        let state = match mode {
            ansi::Mode::Named(mode) => match mode {
                NamedMode::Insert => self.mode.contains(TermMode::INSERT).into(),
                NamedMode::LineFeedNewLine => {
                    self.mode.contains(TermMode::LINE_FEED_NEW_LINE).into()
                },
            },
            ansi::Mode::Unknown(n) => match self.extended_ansi_mode(n) {
                Some(on) => on.into(),
                None => ModeState::NotSupported,
            },
        };

        self.report_mode_state(false, mode.raw(), state as u8);
    }

    #[inline]
    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        // Fallback to the last line as default.
        let bottom = bottom.unwrap_or_else(|| self.screen_lines());

        if top >= bottom {
            debug!("Invalid scrolling region: ({top};{bottom})");
            return;
        }

        // Bottom should be included in the range, but range end is not
        // usually included. One option would be to use an inclusive
        // range, but instead we just let the open range end be 1
        // higher.
        let start = Line(top as i32 - 1);
        let end = Line(bottom as i32);

        trace!("Setting scrolling region: ({start};{end})");

        let screen_lines = Line(self.screen_lines() as i32);
        self.scroll_region.start = cmp::min(start, screen_lines);
        self.scroll_region.end = cmp::min(end, screen_lines);
        self.goto(0, 0);
    }

    #[inline]
    fn set_keypad_application_mode(&mut self) {
        trace!("Setting keypad application mode");
        self.mode.insert(TermMode::APP_KEYPAD);
    }

    #[inline]
    fn unset_keypad_application_mode(&mut self) {
        trace!("Unsetting keypad application mode");
        self.mode.remove(TermMode::APP_KEYPAD);
    }

    #[inline]
    fn configure_charset(&mut self, index: CharsetIndex, charset: StandardCharset) {
        trace!("Configuring charset {index:?} as {charset:?}");
        self.grid.cursor.charsets[index] = charset;
    }

    #[inline]
    fn set_active_charset(&mut self, index: CharsetIndex) {
        trace!("Setting active charset {index:?}");
        self.active_charset = index;
    }

    #[inline]
    fn set_cursor_style(&mut self, style: Option<CursorStyle>) {
        trace!("Setting cursor style {style:?}");
        self.cursor_style = style;

        // Notify UI about blinking changes.
        self.event_proxy.send_event(Event::CursorBlinkingChange);
    }

    #[inline]
    fn set_cursor_shape(&mut self, shape: CursorShape) {
        trace!("Setting cursor shape {shape:?}");

        let style = self.cursor_style.get_or_insert(self.config.default_cursor_style);
        style.shape = shape;
    }

    #[inline]
    fn set_title(&mut self, title: Option<String>) {
        trace!("Setting title to '{title:?}'");

        self.title.clone_from(&title);

        let title_event = match title {
            Some(title) => Event::Title(title),
            None => Event::ResetTitle,
        };

        self.event_proxy.send_event(title_event);
    }

    #[inline]
    fn push_title(&mut self) {
        trace!("Pushing '{:?}' onto title stack", self.title);

        if self.title_stack.len() >= TITLE_STACK_MAX_DEPTH {
            let removed = self.title_stack.remove(0);
            trace!(
                "Removing '{removed:?}' from bottom of title stack that exceeds its maximum depth"
            );
        }

        self.title_stack.push(self.title.clone());
    }

    #[inline]
    fn pop_title(&mut self) {
        trace!("Attempting to pop title from stack...");

        if let Some(popped) = self.title_stack.pop() {
            trace!("Title '{popped:?}' popped from stack");
            self.set_title(popped);
        }
    }

    #[inline]
    fn text_area_size_pixels(&mut self) {
        self.event_proxy.send_event(Event::TextAreaSizeRequest(Arc::new(move |window_size| {
            let height = window_size.num_lines * window_size.cell_height;
            let width = window_size.num_cols * window_size.cell_width;
            format!("\x1b[4;{height};{width}t")
        })));
    }

    #[inline]
    fn text_area_size_chars(&mut self) {
        let text = format!("\x1b[8;{};{}t", self.screen_lines(), self.columns());
        self.reply(text);
    }

    // ── VT420 / xterm extensions (Rift); see `vt420.rs` ──

    #[inline]
    fn request_checksum(&mut self, id: u16, rect: ansi::Rect) {
        let sum = self.resolve_rect(rect).map_or(0, |area| self.area_checksum(area));
        self.reply(format!("\x1bP{id}!~{sum:04X}\x1b\\"));
    }

    #[inline]
    fn dcs_request(&mut self, request: ansi::DcsRequest) {
        match request {
            ansi::DcsRequest::StatusString(pt) => self.report_status_string(&pt),
            ansi::DcsRequest::Termcap(pt) => self.report_termcap(&pt),
            ansi::DcsRequest::RestorePresentation(kind, data) => {
                self.restore_presentation(kind, &data)
            },
            ansi::DcsRequest::UserDefinedKeys => {},
        }
    }

    #[inline]
    fn dec_device_status(&mut self, n: u16, arg: u16) {
        let text = match n {
            // DECXCPR: cursor position and page
            6 => {
                let (row, col) = self.cursor_report();
                format!("\x1b[?{row};{col};1R")
            },
            // printer: none connected
            15 => String::from("\x1b[?13n"),
            // user-defined keys: unlocked
            25 => String::from("\x1b[?20n"),
            // keyboard: North American, ready, LK201
            26 => String::from("\x1b[?27;1;0;0n"),
            // locator: ready (the mouse)
            53 | 55 => String::from("\x1b[?53n"),
            // locator type: mouse
            56 => String::from("\x1b[?57;1n"),
            // macro space: none
            62 => String::from("\x1b[0*{"),
            // memory checksum, tagged with the request's id
            63 => format!("\x1bP{arg}!~0000\x1b\\"),
            // data integrity: no errors
            75 => String::from("\x1b[?70n"),
            // multiple sessions: not configured
            85 => String::from("\x1b[?83n"),
            _ => return self.ignore("DECDSR"),
        };
        self.reply(text);
    }

    #[inline]
    fn report_version(&mut self) {
        let text = format!("\x1bP>|{}\x1b\\", self.config.xtversion);
        self.reply(text);
    }

    #[inline]
    fn window_op(&mut self, params: &[Option<u16>]) {
        self.do_window_op(params);
    }

    #[inline]
    fn csi_s(&mut self, left: u16, right: u16) {
        // DECSLRM needs DECLRMM and a VT420 conformance level
        if !self.vt.declrmm || self.vt.level < 64 {
            self.save_cursor_position();
            return;
        }
        let cols = self.columns();
        let left = if left == 0 { 1 } else { left as usize };
        let right = if right == 0 { cols } else { cmp::min(right as usize, cols) };
        if left >= right {
            return;
        }
        self.vt.left = left - 1;
        self.vt.right = right - 1;
        self.goto(0, 0);
    }

    #[inline]
    fn copy_rect(&mut self, src: ansi::Rect, top: u16, left: u16) {
        self.do_copy_rect(src, top, left);
    }

    #[inline]
    fn fill_rect(&mut self, c: char, rect: ansi::Rect) {
        self.do_fill_rect(c, rect);
    }

    #[inline]
    fn erase_rect(&mut self, rect: ansi::Rect, selective: bool) {
        if let Some(area) = self.resolve_rect(rect) {
            self.erase_area(area, selective);
        }
    }

    #[inline]
    fn change_rect_attrs(&mut self, rect: ansi::Rect, attrs: &[u16], reverse: bool) {
        self.do_change_rect_attrs(rect, attrs, reverse);
    }

    #[inline]
    fn set_attr_change_extent(&mut self, rectangle: bool) {
        self.vt.rect_extent = rectangle;
    }

    #[inline]
    fn set_attr_change_extent_raw(&mut self, n: u16) {
        self.vt.sace = n;
    }

    #[inline]
    fn save_private_modes(&mut self, modes: &[u16]) {
        for &mode in modes {
            if let Some(on) = self.private_mode_state(mode) {
                self.vt.saved_modes.insert(mode, on);
            }
        }
    }

    #[inline]
    fn restore_private_modes(&mut self, modes: &[u16]) {
        for &mode in modes {
            if let Some(&on) = self.vt.saved_modes.get(&mode) {
                let mode = PrivateMode::new(mode);
                if on { self.set_private_mode(mode) } else { self.unset_private_mode(mode) }
            }
        }
    }

    #[inline]
    fn insert_columns(&mut self, n: usize) {
        if !self.cursor_in_tb() || !self.cursor_in_lr() {
            return;
        }
        let (top, bottom) = self.tb();
        let right = self.lr().1;
        let left = self.grid.cursor.point.column.0;
        self.shift_area(vt420::Area { top, left, bottom, right }, 0, n as isize);
    }

    #[inline]
    fn delete_columns(&mut self, n: usize) {
        if !self.cursor_in_tb() || !self.cursor_in_lr() {
            return;
        }
        let (top, bottom) = self.tb();
        let right = self.lr().1;
        let left = self.grid.cursor.point.column.0;
        self.shift_area(vt420::Area { top, left, bottom, right }, 0, -(n as isize));
    }

    #[inline]
    fn set_protection(&mut self, protected: bool) {
        self.grid.cursor.template.flags.set(Flags::PROTECTED, protected);
        self.vt.iso_mode = false;
    }

    #[inline]
    fn selective_erase_display(&mut self, mode: ansi::ClearMode) {
        let point = self.grid.cursor.point;
        let (line, col) = (point.line.0 as usize, point.column.0);
        let last_line = self.screen_lines() - 1;
        let last_col = self.columns() - 1;
        let full = |top, bottom| vt420::Area { top, left: 0, bottom, right: last_col };
        // xterm's DECSED also honours SPA/EPA protection (in ISO mode)
        let both = if self.vt.iso_mode {
            Flags::PROTECTED | Flags::ISO_PROTECTED
        } else {
            Flags::PROTECTED
        };
        match mode {
            ansi::ClearMode::Below => {
                let first = vt420::Area { top: line, left: col, bottom: line, right: last_col };
                self.erase_area_keeping(first, true, both);
                if line < last_line {
                    self.erase_area_keeping(full(line + 1, last_line), true, both);
                }
            },
            ansi::ClearMode::Above => {
                if line > 0 {
                    self.erase_area_keeping(full(0, line - 1), true, both);
                }
                let last = vt420::Area { top: line, left: 0, bottom: line, right: col };
                self.erase_area_keeping(last, true, both);
            },
            ansi::ClearMode::All => self.erase_area_keeping(full(0, last_line), true, both),
            ansi::ClearMode::Saved => {},
        }
    }

    #[inline]
    fn selective_erase_line(&mut self, mode: ansi::LineClearMode) {
        let point = self.grid.cursor.point;
        let (line, col) = (point.line.0 as usize, point.column.0);
        let (left, right) = match mode {
            ansi::LineClearMode::Right => (col, self.columns() - 1),
            ansi::LineClearMode::Left => (0, col),
            ansi::LineClearMode::All => (0, self.columns() - 1),
        };
        let both = if self.vt.iso_mode {
            Flags::PROTECTED | Flags::ISO_PROTECTED
        } else {
            Flags::PROTECTED
        };
        self.erase_area_keeping(vt420::Area { top: line, left, bottom: line, right }, true, both);
    }

    #[inline]
    fn back_index(&mut self) {
        let col = self.grid.cursor.point.column.0;
        let (left, right) = self.lr();
        self.grid.cursor.input_needs_wrap = false;
        if col == left && self.cursor_in_tb() {
            // at the left margin the margin rectangle scrolls right
            let (top, bottom) = self.tb();
            self.shift_area(vt420::Area { top, left, bottom, right }, 0, 1);
        } else if col > 0 {
            self.grid.cursor.point.column -= 1;
        }
    }

    #[inline]
    fn forward_index(&mut self) {
        let col = self.grid.cursor.point.column.0;
        let (left, right) = self.lr();
        self.grid.cursor.input_needs_wrap = false;
        if col == right && self.cursor_in_tb() {
            let (top, bottom) = self.tb();
            self.shift_area(vt420::Area { top, left, bottom, right }, 0, -1);
        } else if col < self.columns() - 1 {
            self.grid.cursor.point.column += 1;
        }
    }

    #[inline]
    fn line_attribute(&mut self, attr: ansi::LineAttribute) {
        let code = match attr {
            ansi::LineAttribute::DoubleHeightTop => 3,
            ansi::LineAttribute::DoubleHeightBottom => 4,
            ansi::LineAttribute::SingleWidth => 0,
            ansi::LineAttribute::DoubleWidth => 6,
        };
        let line = self.grid.cursor.point.line;
        self.grid[line].attr = code;
        if code != 0 {
            self.vt.line_sizes = true;
            // half as many cells fit on the line
            let max = Column(self.columns() / 2 - 1);
            if self.grid.cursor.point.column > max {
                self.grid.cursor.point.column = max;
            }
        }
        self.mark_fully_damaged();
    }

    #[inline]
    fn set_conformance_level(&mut self, level: u16, c1: u16) {
        if !(61..=65).contains(&level) {
            return;
        }
        self.soft_reset();
        self.vt.level = cmp::min(level, 64);
        // VT100 mode has no 8-bit controls; otherwise 0 or 2 selects them
        self.vt.c1_8bit = level > 61 && c1 != 1;
    }

    #[inline]
    fn soft_reset(&mut self) {
        self.do_soft_reset();
    }

    #[inline]
    fn set_c1_mode(&mut self, eight_bit: bool) {
        if self.vt.level > 61 {
            self.vt.c1_8bit = eight_bit;
        }
    }

    #[inline]
    fn set_page_size(&mut self, lines: Option<u16>, columns: Option<u16>) {
        if let Some(lines) = lines {
            self.vt.page_lines = Some(lines);
        }
        let rows = lines.map_or(self.screen_lines(), |l| l as usize);
        let cols = columns.map_or(self.columns(), |c| c as usize);
        self.request_resize(rows, cols);
        self.event_proxy.send_event(Event::WindowOp(vec![Some(8), lines, columns]));
    }

    #[inline]
    fn request_presentation_state(&mut self, n: u16) {
        self.do_presentation_report(n);
    }

    #[inline]
    fn request_terminal_state(&mut self, n: u16) {
        self.do_terminal_state_report(n);
    }

    #[inline]
    fn locator(&mut self, op: char, params: &[u16]) {
        self.do_locator(op, params);
    }

    #[inline]
    fn status_line(&mut self, op: char, n: u16) {
        // DECSASD (`$}`, active display) and DECSSDT (`$~`, status line
        // type): there is no status line; the setting is kept and reported
        match op {
            '}' => self.vt.status_display = n,
            _ => self.vt.status_type = n,
        }
    }

    #[inline]
    fn set_icon_name(&mut self, name: Option<String>) {
        self.vt.icon = name;
    }

    #[inline]
    fn protected_area(&mut self, start: bool) {
        self.vt.iso_protect = start;
        if start {
            self.vt.iso_mode = true;
        }
        self.grid.cursor.template.flags.set(Flags::ISO_PROTECTED, start);
    }

    #[inline]
    fn set_special_color(&mut self, index: usize, color: Rgb) {
        if let Some(slot) = self.vt.special.get_mut(index) {
            *slot = Some(color);
        }
    }

    #[inline]
    fn special_color_query(&mut self, prefix: String, index: usize, terminator: &str) {
        let terminator = terminator.to_owned();
        let format = move |c: Rgb| {
            format!(
                "\x1b]{prefix};rgb:{0:02x}{0:02x}/{1:02x}{1:02x}/{2:02x}{2:02x}{terminator}",
                c.r, c.g, c.b
            )
        };
        match self.vt.special.get(index).copied().flatten() {
            Some(color) => self.reply(format(color)),
            // unset: the special colors default to the foreground
            None => self
                .event_proxy
                .send_event(Event::ColorRequest(NamedColor::Foreground as usize, Arc::new(format))),
        }
    }

    #[inline]
    fn reset_special_color(&mut self, index: usize) {
        if let Some(slot) = self.vt.special.get_mut(index) {
            *slot = None;
        }
    }
}

/// The state of the [`Mode`] and [`PrivateMode`].
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
enum ModeState {
    /// The mode is not supported.
    NotSupported = 0,
    /// The mode is currently set.
    Set = 1,
    /// The mode is currently not set.
    Reset = 2,
}

impl From<bool> for ModeState {
    fn from(value: bool) -> Self {
        if value { Self::Set } else { Self::Reset }
    }
}

/// Terminal version for escape sequence reports.
///
/// This returns the current terminal version as a unique number based on alacritty_terminal's
/// semver version. The different versions are padded to ensure that a higher semver version will
/// always report a higher version number.
fn version_number(mut version: &str) -> usize {
    if let Some(separator) = version.rfind('-') {
        version = &version[..separator];
    }

    let mut version_number = 0;

    let semver_versions = version.split('.');
    for (i, semver_version) in semver_versions.rev().enumerate() {
        let semver_number = semver_version.parse::<usize>().unwrap_or(0);
        version_number += usize::pow(100, i as u32) * semver_number;
    }

    version_number
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardType {
    Clipboard,
    Selection,
}

struct TabStops {
    tabs: Vec<bool>,
}

impl TabStops {
    #[inline]
    fn new(columns: usize) -> TabStops {
        TabStops { tabs: (0..columns).map(|i| i % INITIAL_TABSTOPS == 0).collect() }
    }

    /// Remove all tabstops.
    #[inline]
    fn clear_all(&mut self) {
        unsafe {
            ptr::write_bytes(self.tabs.as_mut_ptr(), 0, self.tabs.len());
        }
    }

    /// Increase tabstop capacity.
    #[inline]
    fn resize(&mut self, columns: usize) {
        let mut index = self.tabs.len();
        self.tabs.resize_with(columns, || {
            let is_tabstop = index % INITIAL_TABSTOPS == 0;
            index += 1;
            is_tabstop
        });
    }
}

impl Index<Column> for TabStops {
    type Output = bool;

    fn index(&self, index: Column) -> &bool {
        &self.tabs[index.0]
    }
}

impl IndexMut<Column> for TabStops {
    fn index_mut(&mut self, index: Column) -> &mut bool {
        self.tabs.index_mut(index.0)
    }
}

/// Terminal cursor rendering information.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct RenderableCursor {
    pub shape: CursorShape,
    pub point: Point,
}

impl RenderableCursor {
    fn new<T>(term: &Term<T>) -> Self {
        // Cursor position.
        let vi_mode = term.mode().contains(TermMode::VI);
        let mut point = if vi_mode { term.vi_mode_cursor.point } else { term.grid.cursor.point };
        if term.grid[point].flags.contains(Flags::WIDE_CHAR_SPACER) {
            point.column -= 1;
        }

        // Cursor shape.
        let shape = if !vi_mode && !term.mode().contains(TermMode::SHOW_CURSOR) {
            CursorShape::Hidden
        } else {
            term.cursor_style().shape
        };

        Self { shape, point }
    }
}

/// Visible terminal content.
///
/// This contains all content required to render the current terminal view.
pub struct RenderableContent<'a> {
    pub display_iter: GridIterator<'a, Cell>,
    pub selection: Option<SelectionRange>,
    pub cursor: RenderableCursor,
    pub display_offset: usize,
    pub colors: &'a color::Colors,
    pub mode: TermMode,
}

impl<'a> RenderableContent<'a> {
    fn new<T>(term: &'a Term<T>) -> Self {
        Self {
            display_iter: term.grid().display_iter(),
            display_offset: term.grid().display_offset(),
            cursor: RenderableCursor::new(term),
            selection: term.selection.as_ref().and_then(|s| s.to_range(term)),
            colors: &term.colors,
            mode: *term.mode(),
        }
    }
}

/// Terminal test helpers.
pub mod test {
    use super::*;
    use unicode_width::UnicodeWidthChar;

    #[cfg(feature = "serde")]
    use serde::{Deserialize, Serialize};

    use crate::event::VoidListener;

    #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
    pub struct TermSize {
        pub columns: usize,
        pub screen_lines: usize,
    }

    impl TermSize {
        pub fn new(columns: usize, screen_lines: usize) -> Self {
            Self { columns, screen_lines }
        }
    }

    impl Dimensions for TermSize {
        fn total_lines(&self) -> usize {
            self.screen_lines()
        }

        fn screen_lines(&self) -> usize {
            self.screen_lines
        }

        fn columns(&self) -> usize {
            self.columns
        }
    }

    /// Construct a terminal from its content as string.
    ///
    /// A `\n` will break line and `\r\n` will break line without wrapping.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use alacritty_terminal::term::test::mock_term;
    ///
    /// // Create a terminal with the following cells:
    /// //
    /// // [h][e][l][l][o] <- WRAPLINE flag set
    /// // [:][)][ ][ ][ ]
    /// // [t][e][s][t][ ]
    /// mock_term(
    ///     "\
    ///     hello\n:)\r\ntest",
    /// );
    /// ```
    pub fn mock_term(content: &str) -> Term<VoidListener> {
        let lines: Vec<&str> = content.split('\n').collect();
        let num_cols = lines
            .iter()
            .map(|line| line.chars().filter(|c| *c != '\r').map(|c| c.width().unwrap()).sum())
            .max()
            .unwrap_or(0);

        // Create terminal with the appropriate dimensions.
        let size = TermSize::new(num_cols, lines.len());
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Fill terminal with content.
        for (line, text) in lines.iter().enumerate() {
            let line = Line(line as i32);
            if !text.ends_with('\r') && line + 1 != lines.len() {
                term.grid[line][Column(num_cols - 1)].flags.insert(Flags::WRAPLINE);
            }

            let mut index = 0;
            for c in text.chars().take_while(|c| *c != '\r') {
                term.grid[line][Column(index)].c = c;

                // Handle fullwidth characters.
                let width = c.width().unwrap();
                if width == 2 {
                    term.grid[line][Column(index)].flags.insert(Flags::WIDE_CHAR);
                    term.grid[line][Column(index + 1)].flags.insert(Flags::WIDE_CHAR_SPACER);
                }

                index += width;
            }
        }

        term
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::mem;

    use crate::event::VoidListener;
    use crate::grid::{Grid, Scroll};
    use crate::index::{Column, Point, Side};
    use crate::selection::{Selection, SelectionType};
    use crate::term::cell::{Cell, Flags};
    use crate::term::test::TermSize;
    use crate::vte::ansi::{self, CharsetIndex, Handler, StandardCharset};

    fn type_str(term: &mut Term<VoidListener>, s: &str) {
        for c in s.chars() {
            term.input(c);
        }
    }

    #[test]
    fn vs16_widens_the_emoji_before_it() {
        let size = TermSize::new(5, 2);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        type_str(&mut term, "\u{2764}\u{fe0f}x");
        let row = &term.grid[Line(0)];
        assert!(row[Column(0)].flags.contains(Flags::WIDE_CHAR));
        assert!(row[Column(1)].flags.contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(row[Column(0)].zerowidth(), Some(&['\u{fe0f}'][..]));
        assert_eq!(row[Column(2)].c, 'x');
    }

    #[test]
    fn vs16_at_the_last_column_moves_the_emoji_down() {
        let size = TermSize::new(3, 2);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        type_str(&mut term, "ab\u{2764}\u{fe0f}");
        assert!(term.grid[Line(0)][Column(2)].flags.contains(Flags::LEADING_WIDE_CHAR_SPACER));
        assert_eq!(term.grid[Line(1)][Column(0)].c, '\u{2764}');
        assert!(term.grid[Line(1)][Column(0)].flags.contains(Flags::WIDE_CHAR));
        assert_eq!(term.grid.cursor.point, Point::new(Line(1), Column(2)));
    }

    #[test]
    fn vs15_narrows_and_zwj_joins() {
        let size = TermSize::new(8, 2);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        type_str(&mut term, "\u{231a}\u{fe0e}x");
        assert!(!term.grid[Line(0)][Column(0)].flags.contains(Flags::WIDE_CHAR));
        assert_eq!(term.grid[Line(0)][Column(1)].c, 'x');

        term.carriage_return();
        term.linefeed();
        type_str(&mut term, "\u{1f468}\u{200d}\u{1f469}y");
        assert_eq!(term.grid[Line(1)][Column(2)].c, 'y');
    }

    #[test]
    fn termcap_advertises_the_underlines_drawn() {
        use crate::event::Event;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Replies(Arc<Mutex<Vec<String>>>);
        impl EventListener for Replies {
            fn send_event(&self, event: Event) {
                if let Event::PtyWrite(text) = event {
                    self.0.lock().unwrap().push(text);
                }
            }
        }

        let replies = Replies::default();
        let size = TermSize::new(8, 2);
        let mut term = Term::new(Config::default(), &size, replies.clone());
        let mut parser: ansi::Processor = ansi::Processor::new();
        // XTGETTCAP Smulx;Setulc
        parser.advance(&mut term, b"\x1bP+q536D756C78;536574756C63\x1b\\");
        let got = replies.0.lock().unwrap().clone();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.iter().all(|r| r.starts_with("\x1bP1+r")), "{got:?}");

        // the advertised Setulc form, expanded for 0x123456, colors the
        // underline
        parser.advance(&mut term, b"\x1b[58:2::18:52:86mx");
        let cell = &term.grid[Line(0)][Column(0)];
        assert_eq!(
            cell.underline_color(),
            Some(ansi::Color::Spec(ansi::Rgb { r: 0x12, g: 0x34, b: 0x56 }))
        );
    }

    #[test]
    fn kitty_keyboard_set_flags_are_reported() {
        use crate::event::Event;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Replies(Arc<Mutex<Vec<String>>>);
        impl EventListener for Replies {
            fn send_event(&self, event: Event) {
                if let Event::PtyWrite(text) = event {
                    self.0.lock().unwrap().push(text);
                }
            }
        }

        let replies = Replies::default();
        let size = TermSize::new(8, 2);
        let config = Config { kitty_keyboard: true, ..Config::default() };
        let mut term = Term::new(config, &size, replies.clone());
        Handler::set_keyboard_mode(
            &mut term,
            KeyboardModes::DISAMBIGUATE_ESC_CODES,
            ansi::KeyboardModesApplyBehavior::Replace,
        );
        term.report_keyboard_mode();
        term.push_keyboard_mode(KeyboardModes::REPORT_EVENT_TYPES);
        Handler::set_keyboard_mode(
            &mut term,
            KeyboardModes::REPORT_ALTERNATE_KEYS,
            ansi::KeyboardModesApplyBehavior::Union,
        );
        term.report_keyboard_mode();
        term.pop_keyboard_modes(1);
        term.report_keyboard_mode();
        assert_eq!(*replies.0.lock().unwrap(), ["\x1b[?1u", "\x1b[?6u", "\x1b[?0u"]);
    }

    #[test]
    fn a_cursor_motion_ends_the_cluster() {
        let size = TermSize::new(8, 2);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        type_str(&mut term, "\u{2764}");
        term.goto(1, 0);
        type_str(&mut term, "\u{fe0f}");
        assert!(!term.grid[Line(0)][Column(0)].flags.contains(Flags::WIDE_CHAR));
    }

    #[test]
    fn scroll_display_page_up() {
        let size = TermSize::new(5, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 11 lines of scrollback.
        for _ in 0..20 {
            term.newline();
        }

        // Scrollable amount to top is 11.
        term.scroll_display(Scroll::PageUp);
        assert_eq!(term.vi_mode_cursor.point, Point::new(Line(-1), Column(0)));
        assert_eq!(term.grid.display_offset(), 10);

        // Scrollable amount to top is 1.
        term.scroll_display(Scroll::PageUp);
        assert_eq!(term.vi_mode_cursor.point, Point::new(Line(-2), Column(0)));
        assert_eq!(term.grid.display_offset(), 11);

        // Scrollable amount to top is 0.
        term.scroll_display(Scroll::PageUp);
        assert_eq!(term.vi_mode_cursor.point, Point::new(Line(-2), Column(0)));
        assert_eq!(term.grid.display_offset(), 11);
    }

    #[test]
    fn scroll_display_page_down() {
        let size = TermSize::new(5, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 11 lines of scrollback.
        for _ in 0..20 {
            term.newline();
        }

        // Change display_offset to topmost.
        term.grid_mut().scroll_display(Scroll::Top);
        term.vi_mode_cursor = ViModeCursor::new(Point::new(Line(-11), Column(0)));

        // Scrollable amount to bottom is 11.
        term.scroll_display(Scroll::PageDown);
        assert_eq!(term.vi_mode_cursor.point, Point::new(Line(-1), Column(0)));
        assert_eq!(term.grid.display_offset(), 1);

        // Scrollable amount to bottom is 1.
        term.scroll_display(Scroll::PageDown);
        assert_eq!(term.vi_mode_cursor.point, Point::new(Line(0), Column(0)));
        assert_eq!(term.grid.display_offset(), 0);

        // Scrollable amount to bottom is 0.
        term.scroll_display(Scroll::PageDown);
        assert_eq!(term.vi_mode_cursor.point, Point::new(Line(0), Column(0)));
        assert_eq!(term.grid.display_offset(), 0);
    }

    #[test]
    fn simple_selection_works() {
        let size = TermSize::new(5, 5);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let grid = term.grid_mut();
        for i in 0..4 {
            if i == 1 {
                continue;
            }

            grid[Line(i)][Column(0)].c = '"';

            for j in 1..4 {
                grid[Line(i)][Column(j)].c = 'a';
            }

            grid[Line(i)][Column(4)].c = '"';
        }
        grid[Line(2)][Column(0)].c = ' ';
        grid[Line(2)][Column(4)].c = ' ';
        grid[Line(2)][Column(4)].flags.insert(Flags::WRAPLINE);
        grid[Line(3)][Column(0)].c = ' ';

        // Multiple lines contain an empty line.
        term.selection = Some(Selection::new(
            SelectionType::Simple,
            Point { line: Line(0), column: Column(0) },
            Side::Left,
        ));
        if let Some(s) = term.selection.as_mut() {
            s.update(Point { line: Line(2), column: Column(4) }, Side::Right);
        }
        assert_eq!(term.selection_to_string(), Some(String::from("\"aaa\"\n\n aaa ")));

        // A wrapline.
        term.selection = Some(Selection::new(
            SelectionType::Simple,
            Point { line: Line(2), column: Column(0) },
            Side::Left,
        ));
        if let Some(s) = term.selection.as_mut() {
            s.update(Point { line: Line(3), column: Column(4) }, Side::Right);
        }
        assert_eq!(term.selection_to_string(), Some(String::from(" aaa  aaa\"")));
    }

    #[test]
    fn semantic_selection_works() {
        let size = TermSize::new(5, 3);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut grid: Grid<Cell> = Grid::new(3, 5, 0);
        for i in 0..5 {
            for j in 0..2 {
                grid[Line(j)][Column(i)].c = 'a';
            }
        }
        grid[Line(0)][Column(0)].c = '"';
        grid[Line(0)][Column(3)].c = '"';
        grid[Line(1)][Column(2)].c = '"';
        grid[Line(0)][Column(4)].flags.insert(Flags::WRAPLINE);

        let mut escape_chars = String::from("\"");

        mem::swap(&mut term.grid, &mut grid);
        mem::swap(&mut term.config.semantic_escape_chars, &mut escape_chars);

        {
            term.selection = Some(Selection::new(
                SelectionType::Semantic,
                Point { line: Line(0), column: Column(1) },
                Side::Left,
            ));
            assert_eq!(term.selection_to_string(), Some(String::from("aa")));
        }

        {
            term.selection = Some(Selection::new(
                SelectionType::Semantic,
                Point { line: Line(0), column: Column(4) },
                Side::Left,
            ));
            assert_eq!(term.selection_to_string(), Some(String::from("aaa")));
        }

        {
            term.selection = Some(Selection::new(
                SelectionType::Semantic,
                Point { line: Line(1), column: Column(1) },
                Side::Left,
            ));
            assert_eq!(term.selection_to_string(), Some(String::from("aaa")));
        }
    }

    #[test]
    fn line_selection_works() {
        let size = TermSize::new(5, 1);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut grid: Grid<Cell> = Grid::new(1, 5, 0);
        for i in 0..5 {
            grid[Line(0)][Column(i)].c = 'a';
        }
        grid[Line(0)][Column(0)].c = '"';
        grid[Line(0)][Column(3)].c = '"';

        mem::swap(&mut term.grid, &mut grid);

        term.selection = Some(Selection::new(
            SelectionType::Lines,
            Point { line: Line(0), column: Column(3) },
            Side::Left,
        ));
        assert_eq!(term.selection_to_string(), Some(String::from("\"aa\"a\n")));
    }

    #[test]
    fn block_selection_works() {
        let size = TermSize::new(5, 5);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let grid = term.grid_mut();
        for i in 1..4 {
            grid[Line(i)][Column(0)].c = '"';

            for j in 1..4 {
                grid[Line(i)][Column(j)].c = 'a';
            }

            grid[Line(i)][Column(4)].c = '"';
        }
        grid[Line(2)][Column(2)].c = ' ';
        grid[Line(2)][Column(4)].flags.insert(Flags::WRAPLINE);
        grid[Line(3)][Column(4)].c = ' ';

        term.selection = Some(Selection::new(
            SelectionType::Block,
            Point { line: Line(0), column: Column(3) },
            Side::Left,
        ));

        // The same column.
        if let Some(s) = term.selection.as_mut() {
            s.update(Point { line: Line(3), column: Column(3) }, Side::Right);
        }
        assert_eq!(term.selection_to_string(), Some(String::from("\na\na\na")));

        // The first column.
        if let Some(s) = term.selection.as_mut() {
            s.update(Point { line: Line(3), column: Column(0) }, Side::Left);
        }
        assert_eq!(term.selection_to_string(), Some(String::from("\n\"aa\n\"a\n\"aa")));

        // The last column.
        if let Some(s) = term.selection.as_mut() {
            s.update(Point { line: Line(3), column: Column(4) }, Side::Right);
        }
        assert_eq!(term.selection_to_string(), Some(String::from("\na\"\na\"\na")));
    }

    /// Check that the grid can be serialized back and forth losslessly.
    ///
    /// This test is in the term module as opposed to the grid since we want to
    /// test this property with a T=Cell.
    #[test]
    #[cfg(feature = "serde")]
    fn grid_serde() {
        let grid: Grid<Cell> = Grid::new(24, 80, 0);
        let serialized = serde_json::to_string(&grid).expect("ser");
        let deserialized = serde_json::from_str::<Grid<Cell>>(&serialized).expect("de");

        assert_eq!(deserialized, grid);
    }

    #[test]
    fn input_line_drawing_character() {
        let size = TermSize::new(7, 17);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let cursor = Point::new(Line(0), Column(0));
        term.configure_charset(CharsetIndex::G0, StandardCharset::SpecialCharacterAndLineDrawing);
        term.input('a');

        assert_eq!(term.grid()[cursor].c, '▒');
    }

    #[test]
    fn clearing_viewport_keeps_history_position() {
        let size = TermSize::new(10, 20);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 10 lines of scrollback.
        for _ in 0..29 {
            term.newline();
        }

        // Change the display area.
        term.scroll_display(Scroll::Top);

        assert_eq!(term.grid.display_offset(), 10);

        // Clear the viewport.
        term.clear_screen(ansi::ClearMode::All);

        assert_eq!(term.grid.display_offset(), 10);
    }

    #[test]
    fn clearing_viewport_with_vi_mode_keeps_history_position() {
        let size = TermSize::new(10, 20);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 10 lines of scrollback.
        for _ in 0..29 {
            term.newline();
        }

        // Enable vi mode.
        term.toggle_vi_mode();

        // Change the display area and the vi cursor position.
        term.scroll_display(Scroll::Top);
        term.vi_mode_cursor.point = Point::new(Line(-5), Column(3));

        assert_eq!(term.grid.display_offset(), 10);

        // Clear the viewport.
        term.clear_screen(ansi::ClearMode::All);

        assert_eq!(term.grid.display_offset(), 10);
        assert_eq!(term.vi_mode_cursor.point, Point::new(Line(-5), Column(3)));
    }

    #[test]
    fn clearing_scrollback_resets_display_offset() {
        let size = TermSize::new(10, 20);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 10 lines of scrollback.
        for _ in 0..29 {
            term.newline();
        }

        // Change the display area.
        term.scroll_display(Scroll::Top);

        assert_eq!(term.grid.display_offset(), 10);

        // Clear the scrollback buffer.
        term.clear_screen(ansi::ClearMode::Saved);

        assert_eq!(term.grid.display_offset(), 0);
    }

    #[test]
    fn clearing_scrollback_sets_vi_cursor_into_viewport() {
        let size = TermSize::new(10, 20);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 10 lines of scrollback.
        for _ in 0..29 {
            term.newline();
        }

        // Enable vi mode.
        term.toggle_vi_mode();

        // Change the display area and the vi cursor position.
        term.scroll_display(Scroll::Top);
        term.vi_mode_cursor.point = Point::new(Line(-5), Column(3));

        assert_eq!(term.grid.display_offset(), 10);

        // Clear the scrollback buffer.
        term.clear_screen(ansi::ClearMode::Saved);

        assert_eq!(term.grid.display_offset(), 0);
        assert_eq!(term.vi_mode_cursor.point, Point::new(Line(0), Column(3)));
    }

    #[test]
    fn clear_saved_lines() {
        let size = TermSize::new(7, 17);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Add one line of scrollback.
        term.grid.scroll_up(&(Line(0)..Line(1)), 1);

        // Clear the history.
        term.clear_screen(ansi::ClearMode::Saved);

        // Make sure that scrolling does not change the grid.
        let mut scrolled_grid = term.grid.clone();
        scrolled_grid.scroll_display(Scroll::Top);

        // Truncate grids for comparison.
        scrolled_grid.truncate();
        term.grid.truncate();

        assert_eq!(term.grid, scrolled_grid);
    }

    #[test]
    fn vi_cursor_keep_pos_on_scrollback_buffer() {
        let size = TermSize::new(5, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 11 lines of scrollback.
        for _ in 0..20 {
            term.newline();
        }

        // Enable vi mode.
        term.toggle_vi_mode();

        term.scroll_display(Scroll::Top);
        term.vi_mode_cursor.point.line = Line(-11);

        term.linefeed();
        assert_eq!(term.vi_mode_cursor.point.line, Line(-12));
    }

    #[test]
    fn grow_lines_updates_active_cursor_pos() {
        let mut size = TermSize::new(100, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 10 lines of scrollback.
        for _ in 0..19 {
            term.newline();
        }
        assert_eq!(term.history_size(), 10);
        assert_eq!(term.grid.cursor.point, Point::new(Line(9), Column(0)));

        // Increase visible lines.
        size.screen_lines = 30;
        term.resize(size);

        assert_eq!(term.history_size(), 0);
        assert_eq!(term.grid.cursor.point, Point::new(Line(19), Column(0)));
    }

    #[test]
    fn grow_lines_updates_inactive_cursor_pos() {
        let mut size = TermSize::new(100, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 10 lines of scrollback.
        for _ in 0..19 {
            term.newline();
        }
        assert_eq!(term.history_size(), 10);
        assert_eq!(term.grid.cursor.point, Point::new(Line(9), Column(0)));

        // Enter alt screen.
        term.set_private_mode(NamedPrivateMode::SwapScreenAndSetRestoreCursor.into());

        // Increase visible lines.
        size.screen_lines = 30;
        term.resize(size);

        // Leave alt screen.
        term.unset_private_mode(NamedPrivateMode::SwapScreenAndSetRestoreCursor.into());

        assert_eq!(term.history_size(), 0);
        assert_eq!(term.grid.cursor.point, Point::new(Line(19), Column(0)));
    }

    #[test]
    fn shrink_lines_updates_active_cursor_pos() {
        let mut size = TermSize::new(100, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 10 lines of scrollback.
        for _ in 0..19 {
            term.newline();
        }
        assert_eq!(term.history_size(), 10);
        assert_eq!(term.grid.cursor.point, Point::new(Line(9), Column(0)));

        // Increase visible lines.
        size.screen_lines = 5;
        term.resize(size);

        assert_eq!(term.history_size(), 15);
        assert_eq!(term.grid.cursor.point, Point::new(Line(4), Column(0)));
    }

    #[test]
    fn shrink_lines_updates_inactive_cursor_pos() {
        let mut size = TermSize::new(100, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Create 10 lines of scrollback.
        for _ in 0..19 {
            term.newline();
        }
        assert_eq!(term.history_size(), 10);
        assert_eq!(term.grid.cursor.point, Point::new(Line(9), Column(0)));

        // Enter alt screen.
        term.set_private_mode(NamedPrivateMode::SwapScreenAndSetRestoreCursor.into());

        // Increase visible lines.
        size.screen_lines = 5;
        term.resize(size);

        // Leave alt screen.
        term.unset_private_mode(NamedPrivateMode::SwapScreenAndSetRestoreCursor.into());

        assert_eq!(term.history_size(), 15);
        assert_eq!(term.grid.cursor.point, Point::new(Line(4), Column(0)));
    }

    #[test]
    fn damage_public_usage() {
        let size = TermSize::new(10, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        // Reset terminal for partial damage tests since it's initialized as fully damaged.
        term.reset_damage();

        // Test that we damage input form [`Term::input`].

        let left = term.grid.cursor.point.column.0;
        term.input('d');
        term.input('a');
        term.input('m');
        term.input('a');
        term.input('g');
        term.input('e');
        let right = term.grid.cursor.point.column.0;

        let mut damaged_lines = match term.damage() {
            TermDamage::Full => panic!("Expected partial damage, however got Full"),
            TermDamage::Partial(damaged_lines) => damaged_lines,
        };
        assert_eq!(damaged_lines.next(), Some(LineDamageBounds { line: 0, left, right }));
        assert_eq!(damaged_lines.next(), None);
        term.reset_damage();

        // Create scrollback.
        for _ in 0..20 {
            term.newline();
        }

        match term.damage() {
            TermDamage::Full => (),
            TermDamage::Partial(_) => panic!("Expected Full damage, however got Partial "),
        };
        term.reset_damage();

        term.scroll_display(Scroll::Delta(10));
        term.reset_damage();

        // No damage when scrolled into viewport.
        for idx in 0..term.columns() {
            term.goto(idx as i32, idx);
        }
        let mut damaged_lines = match term.damage() {
            TermDamage::Full => panic!("Expected partial damage, however got Full"),
            TermDamage::Partial(damaged_lines) => damaged_lines,
        };
        assert_eq!(damaged_lines.next(), None);

        // Scroll back into the viewport, so we have 2 visible lines which terminal can write
        // to.
        term.scroll_display(Scroll::Delta(-2));
        term.reset_damage();

        term.goto(0, 0);
        term.goto(1, 0);
        term.goto(2, 0);
        let display_offset = term.grid().display_offset();
        let mut damaged_lines = match term.damage() {
            TermDamage::Full => panic!("Expected partial damage, however got Full"),
            TermDamage::Partial(damaged_lines) => damaged_lines,
        };
        assert_eq!(
            damaged_lines.next(),
            Some(LineDamageBounds { line: display_offset, left: 0, right: 0 })
        );
        assert_eq!(
            damaged_lines.next(),
            Some(LineDamageBounds { line: display_offset + 1, left: 0, right: 0 })
        );
        assert_eq!(damaged_lines.next(), None);
    }

    #[test]
    fn damage_cursor_movements() {
        let size = TermSize::new(10, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let num_cols = term.columns();
        // Reset terminal for partial damage tests since it's initialized as fully damaged.
        term.reset_damage();

        term.goto(1, 1);

        // NOTE While we can use `[Term::damage]` to access terminal damage information, in the
        // following tests we will be accessing `term.damage.lines` directly to avoid adding extra
        // damage information (like cursor and Vi cursor), which we're not testing.

        assert_eq!(term.damage.lines[0], LineDamageBounds { line: 0, left: 0, right: 0 });
        assert_eq!(term.damage.lines[1], LineDamageBounds { line: 1, left: 1, right: 1 });
        term.damage.reset(num_cols);

        term.move_forward(3);
        assert_eq!(term.damage.lines[1], LineDamageBounds { line: 1, left: 1, right: 4 });
        term.damage.reset(num_cols);

        term.move_backward(8);
        assert_eq!(term.damage.lines[1], LineDamageBounds { line: 1, left: 0, right: 4 });
        term.goto(5, 5);
        term.damage.reset(num_cols);

        term.backspace();
        term.backspace();
        assert_eq!(term.damage.lines[5], LineDamageBounds { line: 5, left: 3, right: 5 });
        term.damage.reset(num_cols);

        term.move_up(1);
        assert_eq!(term.damage.lines[5], LineDamageBounds { line: 5, left: 3, right: 3 });
        assert_eq!(term.damage.lines[4], LineDamageBounds { line: 4, left: 3, right: 3 });
        term.damage.reset(num_cols);

        term.move_down(1);
        term.move_down(1);
        assert_eq!(term.damage.lines[4], LineDamageBounds { line: 4, left: 3, right: 3 });
        assert_eq!(term.damage.lines[5], LineDamageBounds { line: 5, left: 3, right: 3 });
        assert_eq!(term.damage.lines[6], LineDamageBounds { line: 6, left: 3, right: 3 });
        term.damage.reset(num_cols);

        term.wrapline();
        assert_eq!(term.damage.lines[6], LineDamageBounds { line: 6, left: 3, right: 3 });
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 0, right: 0 });
        term.move_forward(3);
        term.move_up(1);
        term.damage.reset(num_cols);

        term.linefeed();
        assert_eq!(term.damage.lines[6], LineDamageBounds { line: 6, left: 3, right: 3 });
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 3, right: 3 });
        term.damage.reset(num_cols);

        term.carriage_return();
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 0, right: 3 });
        term.damage.reset(num_cols);

        term.erase_chars(5);
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 0, right: 5 });
        term.damage.reset(num_cols);

        term.delete_chars(3);
        let right = term.columns() - 1;
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 0, right });
        term.move_forward(term.columns());
        term.damage.reset(num_cols);

        term.move_backward_tabs(1);
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 8, right });
        term.save_cursor_position();
        term.goto(1, 1);
        term.damage.reset(num_cols);

        term.restore_cursor_position();
        assert_eq!(term.damage.lines[1], LineDamageBounds { line: 1, left: 1, right: 1 });
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 8, right: 8 });
        term.damage.reset(num_cols);

        term.clear_line(ansi::LineClearMode::All);
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 0, right });
        term.damage.reset(num_cols);

        term.clear_line(ansi::LineClearMode::Left);
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 0, right: 8 });
        term.damage.reset(num_cols);

        term.clear_line(ansi::LineClearMode::Right);
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 8, right });
        term.damage.reset(num_cols);

        term.reverse_index();
        assert_eq!(term.damage.lines[7], LineDamageBounds { line: 7, left: 8, right: 8 });
        assert_eq!(term.damage.lines[6], LineDamageBounds { line: 6, left: 8, right: 8 });
    }

    #[test]
    fn full_damage() {
        let size = TermSize::new(100, 10);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        assert!(term.damage.full);
        for _ in 0..20 {
            term.newline();
        }
        term.reset_damage();

        term.clear_screen(ansi::ClearMode::Above);
        assert!(term.damage.full);
        term.reset_damage();

        term.scroll_display(Scroll::Top);
        assert!(term.damage.full);
        term.reset_damage();

        // Sequential call to scroll display without doing anything shouldn't damage.
        term.scroll_display(Scroll::Top);
        assert!(!term.damage.full);
        term.reset_damage();

        term.set_options(Config::default());
        assert!(term.damage.full);
        term.reset_damage();

        term.scroll_down_relative(Line(5), 2);
        assert!(term.damage.full);
        term.reset_damage();

        term.scroll_up_relative(Line(3), 2);
        assert!(term.damage.full);
        term.reset_damage();

        // DECCOLM acts only with 80/132 switching allowed (?40, Rift)
        term.set_private_mode(PrivateMode::Unknown(40));
        term.deccolm(true);
        assert!(term.damage.full);
        term.reset_damage();

        term.decaln();
        assert!(term.damage.full);
        term.reset_damage();

        term.set_mode(NamedMode::Insert.into());
        // Just setting `Insert` mode shouldn't mark terminal as damaged.
        assert!(!term.damage.full);
        term.reset_damage();

        let color_index = 257;
        term.set_color(color_index, Rgb::default());
        assert!(term.damage.full);
        term.reset_damage();

        // Setting the same color once again shouldn't trigger full damage.
        term.set_color(color_index, Rgb::default());
        assert!(!term.damage.full);

        term.reset_color(color_index);
        assert!(term.damage.full);
        term.reset_damage();

        // We shouldn't trigger fully damage when cursor gets update.
        term.set_color(NamedColor::Cursor as usize, Rgb::default());
        assert!(!term.damage.full);

        // However requesting terminal damage should mark terminal as fully damaged in `Insert`
        // mode.
        let _ = term.damage();
        assert!(term.damage.full);
        term.reset_damage();

        term.unset_mode(NamedMode::Insert.into());
        assert!(term.damage.full);
        term.reset_damage();

        // Keep this as a last check, so we don't have to deal with restoring from alt-screen.
        term.swap_alt();
        assert!(term.damage.full);
        term.reset_damage();

        let size = TermSize::new(10, 10);
        term.resize(size);
        assert!(term.damage.full);
    }

    #[test]
    fn window_title() {
        let size = TermSize::new(7, 17);
        let mut term = Term::new(Config::default(), &size, VoidListener);

        // Title None by default.
        assert_eq!(term.title, None);

        // Title can be set.
        term.set_title(Some("Test".into()));
        assert_eq!(term.title, Some("Test".into()));

        // Title can be pushed onto stack.
        term.push_title();
        term.set_title(Some("Next".into()));
        assert_eq!(term.title, Some("Next".into()));
        assert_eq!(term.title_stack.first().unwrap(), &Some("Test".into()));

        // Title can be popped from stack and set as the window title.
        term.pop_title();
        assert_eq!(term.title, Some("Test".into()));
        assert!(term.title_stack.is_empty());

        // Title stack doesn't grow infinitely.
        for _ in 0..4097 {
            term.push_title();
        }
        assert_eq!(term.title_stack.len(), 4096);

        // Title and title stack reset when terminal state is reset.
        term.push_title();
        term.reset_state();
        assert_eq!(term.title, None);
        assert!(term.title_stack.is_empty());

        // Title stack pops back to default.
        term.title = None;
        term.push_title();
        term.set_title(Some("Test".into()));
        term.pop_title();
        assert_eq!(term.title, None);

        // Title can be reset to default.
        term.title = Some("Test".into());
        term.set_title(None);
        assert_eq!(term.title, None);
    }

    #[test]
    fn parse_cargo_version() {
        assert!(version_number(env!("CARGO_PKG_VERSION")) >= 10_01);
        assert_eq!(version_number("0.0.1-dev"), 1);
        assert_eq!(version_number("0.1.2-dev"), 1_02);
        assert_eq!(version_number("1.2.3-dev"), 1_02_03);
        assert_eq!(version_number("999.99.99"), 9_99_99_99);
    }
}
