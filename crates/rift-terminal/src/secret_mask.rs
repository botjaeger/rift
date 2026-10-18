//! Secret masking on screen (`mask_secrets`; the patterns are
//! `rift_core::secrets`). A pane's rows are matched as whole lines (a token
//! soft-wrapped over several rows is still found), each line's matches
//! cached by its text; only the rows on screen are read, so the cost of a
//! frame is bounded by the rows shown, never by the size of the output.
//! Rows inside a `-----BEGIN … PRIVATE KEY-----` block are masked whole.
//! A click on a masked span shows it for [`REVEAL_FOR`].

use rift_core::secrets::Masker;
use rift_pty::alacritty_terminal::grid::{Dimensions, Grid};
use rift_pty::alacritty_terminal::index::{Column, Line};
use rift_pty::alacritty_terminal::term::cell::{Cell, Flags};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a clicked secret stays shown.
pub const REVEAL_FOR: Duration = Duration::from_secs(5);
/// Rows of one soft-wrapped line read for matching, at most, each way.
const MAX_WRAP_ROWS: i32 = 8;
/// Rows above the screen searched for the start of a private key block.
const PEM_LOOKBACK: i32 = 128;
/// Lines whose matches are cached per pane (cleared when full).
const CACHE_MAX: usize = 4096;

/// A masked span's identity across frames: the absolute row its line
/// starts on and its first char in that line.
pub type SpanKey = (u64, usize);

/// One masked span on a row: grid columns `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowSpan {
    pub start: usize,
    pub end: usize,
    pub key: SpanKey,
}

/// A pane's masking state.
#[derive(Default)]
pub struct PaneSecrets {
    /// Matches by line text (hash): char spans in the line.
    cache: HashMap<u64, Vec<(usize, usize)>>,
    /// The masked spans drawn last frame: (rect, span).
    pub hits: Vec<([f32; 4], SpanKey)>,
    /// Spans shown on a click, until when.
    pub revealed: Vec<(SpanKey, Instant)>,
}

impl PaneSecrets {
    /// The masked span drawn at (x, y) last frame.
    pub fn hit_at(&self, x: f32, y: f32) -> Option<SpanKey> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
            .map(|(_, k)| *k)
    }

    /// Show `key` for [`REVEAL_FOR`] from `now`.
    pub fn reveal(&mut self, key: SpanKey, now: Instant) {
        self.revealed.retain(|(k, _)| *k != key);
        self.revealed.push((key, now + REVEAL_FOR));
    }

    /// Drop the reveals that ran out: whether any did (a repaint), and when
    /// the next one does.
    pub fn expire(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let before = self.revealed.len();
        self.revealed.retain(|(_, until)| *until > now);
        (
            self.revealed.len() != before,
            self.revealed.iter().map(|(_, u)| *u).min(),
        )
    }

    fn is_revealed(&self, key: SpanKey, now: Instant) -> bool {
        self.revealed.iter().any(|(k, u)| *k == key && *u > now)
    }
}

/// Whether row `line` of `g` continues on the next one (soft wrap).
fn wraps(g: &Grid<Cell>, line: Line) -> bool {
    let cols = g.columns();
    cols > 0 && g[line][Column(cols - 1)].flags.contains(Flags::WRAPLINE)
}

fn in_grid(g: &Grid<Cell>, line: Line) -> bool {
    line.0 >= -(g.history_size() as i32) && line.0 < g.screen_lines() as i32
}

/// A row's text for marker checks.
fn row_text(g: &Grid<Cell>, line: Line) -> String {
    let row = &g[line];
    (0..g.columns()).map(|c| row[Column(c)].c).collect()
}

fn is_begin(text: &str) -> bool {
    text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----")
}

fn is_end(text: &str) -> bool {
    text.contains("-----END ") && text.contains("PRIVATE KEY-----")
}

/// Rows `from..=to` inside a private key block (between its BEGIN and END
/// lines, which the patterns mask themselves), found by reading back at
/// most [`PEM_LOOKBACK`] rows above `from`.
pub fn pem_body(g: &Grid<Cell>, from: Line, to: Line) -> Vec<Line> {
    let top = Line((from.0 - PEM_LOOKBACK).max(-(g.history_size() as i32)));
    let mut inside = false;
    let mut since = 0;
    let mut out = Vec::new();
    let mut l = top;
    while l <= to && in_grid(g, l) {
        let text = row_text(g, l);
        if is_begin(&text) {
            inside = true;
            since = 0;
        } else if is_end(&text) || since > PEM_LOOKBACK {
            inside = false;
        } else if inside {
            since += 1;
            if l >= from {
                out.push(l);
            }
        }
        l = Line(l.0 + 1);
    }
    out
}

/// The soft-wrapped line around `line`: its first row, its text and, per
/// char, (row, column, end column). At most [`MAX_WRAP_ROWS`] rows each way.
type Cells = Vec<(Line, usize, usize)>;
fn logical_line(g: &Grid<Cell>, line: Line) -> (Line, String, Cells) {
    let mut first = line;
    while first.0 > line.0 - MAX_WRAP_ROWS
        && in_grid(g, Line(first.0 - 1))
        && wraps(g, Line(first.0 - 1))
    {
        first = Line(first.0 - 1);
    }
    let mut last = line;
    while last.0 < line.0 + MAX_WRAP_ROWS && wraps(g, last) && in_grid(g, Line(last.0 + 1)) {
        last = Line(last.0 + 1);
    }
    let mut text = String::new();
    let mut cells = Vec::new();
    let mut l = first;
    while l <= last {
        let row = &g[l];
        for col in 0..g.columns() {
            let cell = &row[Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let end = col
                + if cell.flags.contains(Flags::WIDE_CHAR) {
                    2
                } else {
                    1
                };
            // hidden (SGR 8) text is not drawn: nothing to mask
            let c = if cell.flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            text.push(c);
            cells.push((l, col, end));
        }
        l = Line(l.0 + 1);
    }
    (first, text, cells)
}

fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Masked column spans of row `line`, the span keys counted from `base`
/// (the absolute row of `Line(0)`). With a cache, a line already matched is
/// not matched again.
fn line_spans(
    g: &Grid<Cell>,
    line: Line,
    base: i64,
    masker: &Masker,
    cache: Option<&mut HashMap<u64, Vec<(usize, usize)>>>,
) -> Vec<RowSpan> {
    let (first, text, cells) = logical_line(g, line);
    let spans = match cache {
        Some(cache) => {
            let key = hash(&text);
            match cache.get(&key) {
                Some(s) => s.clone(),
                None => {
                    let s = masker.find(&text);
                    if cache.len() >= CACHE_MAX {
                        cache.clear();
                    }
                    cache.insert(key, s.clone());
                    s
                }
            }
        }
        None => masker.find(&text),
    };
    let first_abs = (base + first.0 as i64).max(0) as u64;
    let mut out: Vec<RowSpan> = Vec::new();
    for (s, e) in spans {
        let mut on_row = cells[s.min(cells.len())..e.min(cells.len())]
            .iter()
            .filter(|c| c.0 == line);
        let Some(a) = on_row.next() else {
            continue;
        };
        let end = on_row.next_back().map_or(a.2, |b| b.2);
        out.push(RowSpan {
            start: a.1,
            end,
            key: (first_abs, s),
        });
    }
    out
}

/// The whole row masked: its columns up to the last non-blank one.
fn whole_row(g: &Grid<Cell>, line: Line, key: SpanKey) -> Vec<RowSpan> {
    let row = &g[line];
    (0..g.columns())
        .rev()
        .find(|&c| row[Column(c)].c != ' ')
        .map(|last| RowSpan {
            start: 0,
            end: last + 1,
            key,
        })
        .into_iter()
        .collect()
}

/// One frame's masking of a pane: the rows it draws ask for their spans.
pub struct FrameMasks<'a> {
    masker: &'a Masker,
    secrets: &'a mut PaneSecrets,
    /// The absolute row of `Line(0)` (the session's `scrolled`).
    base: i64,
    pem: Vec<Line>,
    now: Instant,
}

impl<'a> FrameMasks<'a> {
    /// Masking for absolute rows `from..=to` of `g` this frame.
    pub fn new(
        masker: &'a Masker,
        secrets: &'a mut PaneSecrets,
        g: &Grid<Cell>,
        scrolled: u64,
        (from, to): (u64, u64),
        now: Instant,
    ) -> Self {
        let base = scrolled as i64;
        let line = |abs: u64| Line((abs as i64 - base) as i32);
        secrets.hits.clear();
        FrameMasks {
            pem: pem_body(g, line(from), line(to)),
            masker,
            secrets,
            base,
            now,
        }
    }

    /// The masked (not revealed) spans of absolute row `abs`.
    pub fn row(&mut self, g: &Grid<Cell>, abs: u64) -> Vec<RowSpan> {
        let line = Line((abs as i64 - self.base) as i32);
        if !in_grid(g, line) {
            return Vec::new();
        }
        let spans = if self.pem.contains(&line) {
            whole_row(g, line, (abs, 0))
        } else {
            line_spans(
                g,
                line,
                self.base,
                self.masker,
                Some(&mut self.secrets.cache),
            )
        };
        spans
            .into_iter()
            .filter(|s| !self.secrets.is_revealed(s.key, self.now))
            .collect()
    }

    /// A masked span drawn at `rect` (its click target for a reveal).
    pub fn drawn(&mut self, rect: [f32; 4], key: SpanKey) {
        self.secrets.hits.push((rect, key));
    }
}

/// Masked column spans of row `line`, without a cache or reveals (the phone
/// gateway's screens). `pem` = the private-key body rows ([`pem_body`]).
pub fn row_cols(g: &Grid<Cell>, line: Line, masker: &Masker, pem: &[Line]) -> Vec<(usize, usize)> {
    if !in_grid(g, line) {
        return Vec::new();
    }
    let spans = if pem.contains(&line) {
        whole_row(g, line, (0, 0))
    } else {
        line_spans(g, line, 0, masker, None)
    };
    spans.into_iter().map(|s| (s.start, s.end)).collect()
}

/// The selection's text (`text`, as the term copies it) with every secret
/// it touches masked, also one only partly selected: whole secrets inside
/// it are masked by the patterns, and the cut ones at its two ends and the
/// rows of a private key block are masked from the screen's own spans.
pub fn mask_selection<T>(
    term: &rift_pty::alacritty_terminal::term::Term<T>,
    text: &str,
    masker: &Masker,
) -> String {
    let mut out = masker.mask(text).into_owned();
    let Some(range) = term.selection.as_ref().and_then(|s| s.to_range(term)) else {
        return out;
    };
    let g = term.grid();
    let (top, bottom) = (range.start.line, range.end.line);
    let pem = pem_body(g, top, bottom);
    // the ends (with the rows a secret there can wrap over) and key rows
    let mut lines: Vec<Line> = (top.0..=bottom.0.min(top.0 + MAX_WRAP_ROWS))
        .chain((bottom.0 - MAX_WRAP_ROWS).max(top.0)..=bottom.0)
        .map(Line)
        .chain(pem.iter().copied())
        .collect();
    lines.sort_unstable();
    lines.dedup();
    let mut cut: Vec<String> = Vec::new();
    for line in lines {
        let (from, to) = if range.is_block {
            (range.start.column.0, range.end.column.0 + 1)
        } else {
            (
                if line == top { range.start.column.0 } else { 0 },
                if line == bottom {
                    range.end.column.0 + 1
                } else {
                    usize::MAX
                },
            )
        };
        let row = &g[line];
        for (s, e) in row_cols(g, line, masker, &pem) {
            let (a, b) = (s.max(from), e.min(to).min(g.columns()));
            let piece: String = (a..b)
                .map(|c| &row[Column(c)])
                .filter(|c| {
                    !c.flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                })
                .map(|c| c.c)
                .collect();
            if !piece.trim().is_empty() {
                cut.push(piece.trim().to_string());
            }
        }
    }
    // longest first: a shorter piece inside a longer one is masked with it
    cut.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
    for piece in cut {
        let bullets: String =
            std::iter::repeat_n(rift_core::secrets::MASK, piece.chars().count()).collect();
        out = out.replace(&piece, &bullets);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rift_pty::alacritty_terminal::term::{Config, Term, test::TermSize};
    use rift_pty::alacritty_terminal::vte::ansi::Processor;

    struct Quiet;
    impl rift_pty::alacritty_terminal::event::EventListener for Quiet {}

    fn term(cols: usize, rows: usize, bytes: &[u8]) -> Term<Quiet> {
        let mut t = Term::new(Config::default(), &TermSize::new(cols, rows), Quiet);
        let mut p: Processor = Processor::new();
        p.advance(&mut t, bytes);
        t
    }

    fn token() -> String {
        format!("ghp_{}", "FAKE0123".repeat(5))
    }

    fn span(start: usize, end: usize, key: SpanKey) -> RowSpan {
        RowSpan { start, end, key }
    }

    #[test]
    fn a_token_wrapped_over_rows_is_masked_on_each_row() {
        // 20 columns: "id " and the 44-char token run over three rows
        let t = term(20, 6, format!("id {}\r\nnext", token()).as_bytes());
        let masker = Masker::new(&[]).unwrap();
        let mut secrets = PaneSecrets::default();
        let mut f = FrameMasks::new(&masker, &mut secrets, t.grid(), 0, (0, 5), Instant::now());
        assert_eq!(f.row(t.grid(), 0), [span(3, 20, (0, 3))]);
        assert_eq!(f.row(t.grid(), 1), [span(0, 20, (0, 3))]);
        assert_eq!(f.row(t.grid(), 2), [span(0, 7, (0, 3))]);
        assert!(f.row(t.grid(), 3).is_empty());
        // the wrapped line is matched once, then read from the cache
        assert_eq!(secrets.cache.len(), 2);
    }

    #[test]
    fn a_revealed_span_shows_until_it_runs_out() {
        let t = term(80, 4, format!("x {}", token()).as_bytes());
        let masker = Masker::new(&[]).unwrap();
        let mut secrets = PaneSecrets::default();
        let now = Instant::now();
        secrets.reveal((0, 2), now);
        let mut f = FrameMasks::new(&masker, &mut secrets, t.grid(), 0, (0, 3), now);
        assert!(f.row(t.grid(), 0).is_empty());
        let later = now + REVEAL_FOR + Duration::from_millis(1);
        assert_eq!(secrets.expire(later), (true, None));
        let mut f = FrameMasks::new(&masker, &mut secrets, t.grid(), 0, (0, 3), later);
        assert_eq!(f.row(t.grid(), 0).len(), 1);
    }

    #[test]
    fn a_private_key_body_is_masked_whole() {
        let pem = "-----BEGIN PRIVATE KEY-----\r\nMIIEvQIBADANBgkqhkiG\r\nAAAA\r\n-----END PRIVATE KEY-----\r\nplain";
        let t = term(40, 8, pem.as_bytes());
        let masker = Masker::new(&[]).unwrap();
        let mut secrets = PaneSecrets::default();
        let mut f = FrameMasks::new(&masker, &mut secrets, t.grid(), 0, (0, 7), Instant::now());
        let mut cols = |r| {
            f.row(t.grid(), r)
                .iter()
                .map(|s| (s.start, s.end))
                .collect::<Vec<_>>()
        };
        assert_eq!(cols(0), [(0, 27)]);
        assert_eq!(cols(1), [(0, 20)]);
        assert_eq!(cols(2), [(0, 4)]);
        assert!(cols(3).is_empty(), "the END line names no secret");
        assert!(cols(4).is_empty());
        let pem = pem_body(t.grid(), Line(0), Line(7));
        assert_eq!(row_cols(t.grid(), Line(1), &masker, &pem), [(0, 20)]);
        assert!(row_cols(t.grid(), Line(4), &masker, &pem).is_empty());
    }

    #[test]
    fn a_selection_cutting_a_secret_copies_it_masked() {
        use rift_pty::alacritty_terminal::index::{Point, Side};
        use rift_pty::alacritty_terminal::selection::{Selection, SelectionType};
        let mut t = term(80, 4, format!("key {} end\r\nok", token()).as_bytes());
        let masker = Masker::new(&[]).unwrap();
        let select = |t: &mut Term<Quiet>, from: usize, to: usize| {
            let mut s = Selection::new(
                SelectionType::Simple,
                Point::new(Line(0), Column(from)),
                Side::Left,
            );
            s.update(Point::new(Line(0), Column(to)), Side::Right);
            t.selection = Some(s);
            t.selection_to_string().unwrap()
        };
        // the start of the token only: the patterns alone miss it
        let text = select(&mut t, 0, 10);
        assert_eq!(text, "key ghp_FAK");
        assert!(masker.find(&text).is_empty());
        assert_eq!(mask_selection(&t, &text, &masker), "key •••••••");
        // its end, and the word after
        let text = select(&mut t, 40, 51);
        assert_eq!(mask_selection(&t, &text, &masker), "•••••••• end");
        // all of it: masked by the pattern, the rest kept
        let text = select(&mut t, 0, 51);
        let masked = mask_selection(&t, &text, &masker);
        assert!(
            masked.starts_with("key ••••") && masked.ends_with("•••• end"),
            "{masked}"
        );
        assert!(!masked.contains("FAKE"));
    }

    #[test]
    fn a_frame_reads_only_its_rows() {
        // 10 000 lines of output, 50 rows on screen: a frame stays fast
        let mut out = String::new();
        for i in 0..10_000 {
            out.push_str(&format!("line {i} token=abcd{i} {}\r\n", "x".repeat(60)));
        }
        let t = term(120, 50, out.as_bytes());
        let masker = Masker::new(&[]).unwrap();
        let mut secrets = PaneSecrets::default();
        let start = Instant::now();
        let mut f = FrameMasks::new(&masker, &mut secrets, t.grid(), 0, (0, 49), start);
        let masked: usize = (0..50).map(|r| f.row(t.grid(), r).len()).sum();
        let took = start.elapsed();
        assert_eq!(masked, 49, "the last row is blank");
        assert!(took < Duration::from_millis(250), "{took:?}");
    }
}
