//! Compare two finished blocks: a modal over the window with both
//! commands, exit codes, durations and directories, and a plain-text line
//! diff of their outputs (`-` only in the first, `+` only in the second).
//! Opens from a two-block selection's menu ("Compare outputs") or a block's
//! "Compare with previous run" (the newest earlier finished block of this
//! pane with the same exact command in the same local directory).
//!
//! The outputs are snapshotted on the UI thread under the session's locks,
//! each cut at [`MAX_BYTES`] and [`MAX_LINES`]; the diff runs on a worker
//! thread, bounded in edits ([`MAX_EDITS`]) and time ([`DEADLINE`]). A
//! result for an older comparison (the modal closed, another opened) is
//! dropped. Anything incomplete says so: rows gone from scrollback, output
//! cut at a bound, or a diff that gave up. Comparing never runs anything.

use std::time::{Duration, Instant, SystemTime};

use rift_core::{BlockId, Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

/// Most output bytes read per block.
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Most output lines compared per block.
pub const MAX_LINES: usize = 20_000;
/// Most line edits the diff looks for before it shows the changed region
/// whole (bounds its time and memory: O(edits²)).
pub const MAX_EDITS: usize = 2_000;
/// Longest the diff may run.
pub const DEADLINE: Duration = Duration::from_secs(2);
/// Characters a diff row shows before it is cut with `…`.
const ROW_CHARS: usize = 400;

/// One compared block: what it ran, how it ended, and its output snapshot.
#[derive(Debug, Clone, Default)]
pub struct Side {
    pub block: BlockId,
    pub command: String,
    pub exit: Option<i32>,
    pub duration: Option<Duration>,
    pub cwd: Option<String>,
    /// `cwd` is on another host (ssh).
    pub remote: bool,
    /// Replayed from a saved transcript (a previous session).
    pub restored: bool,
    pub finished: Option<SystemTime>,
    pub output: super::terminal::OutputSnapshot,
    /// The block had no output rows at all.
    pub no_output: bool,
}

impl Side {
    /// `exit 0 · 1.20s · ~/proj · 3 min ago · restored`
    pub fn meta(&self, now: SystemTime) -> String {
        let mut parts = vec![match self.exit {
            Some(c) => format!("exit {c}"),
            None => "exit unknown".to_string(),
        }];
        if let Some(d) = self.duration {
            parts.push(super::terminal::format_duration(d));
        }
        if let Some(cwd) = &self.cwd {
            let dir = super::editor::shorten_home(cwd);
            parts.push(if self.remote {
                format!("{dir} (remote host)")
            } else {
                dir
            });
        }
        if let Some(t) = self.finished {
            parts.push(super::history::age(t, now));
        }
        if self.restored {
            parts.push("restored".into());
        }
        parts.join(" · ")
    }

    /// What is missing from this side's output, if anything.
    pub fn gap(&self) -> Option<String> {
        let o = &self.output;
        if !self.no_output && o.total_rows > 0 && o.trimmed_rows >= o.total_rows {
            return Some("output unavailable (trimmed from scrollback)".into());
        }
        let mut gaps = Vec::new();
        if o.trimmed_rows > 0 {
            gaps.push(format!(
                "first {} rows trimmed from scrollback",
                o.trimmed_rows
            ));
        }
        if o.truncated {
            gaps.push(format!(
                "cut at {MAX_LINES} lines / {} MB",
                MAX_BYTES / (1024 * 1024)
            ));
        }
        (!gaps.is_empty()).then(|| gaps.join(", "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Same,
    /// Only in the first block's output.
    Del,
    /// Only in the second block's output.
    Ins,
}

#[derive(Debug, Default, Clone)]
pub struct DiffResult {
    pub lines: Vec<(Op, String)>,
    pub added: usize,
    pub removed: usize,
    /// The edit or time bound was reached: the changed region shows whole
    /// (every line removed, then every line added).
    pub gave_up: bool,
    /// A side had more than [`MAX_LINES`] lines: the rest is not compared.
    pub lines_cut: bool,
}

impl DiffResult {
    pub fn equal(&self) -> bool {
        self.added == 0 && self.removed == 0
    }
}

/// Bounds of one diff.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_lines: usize,
    pub max_edits: usize,
    pub deadline: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_lines: MAX_LINES,
            max_edits: MAX_EDITS,
            deadline: DEADLINE,
        }
    }
}

/// Lines of `text` (no final empty line for a final newline).
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .collect()
}

/// Line diff of `a` → `b` (Myers), within `limits`.
pub fn diff(a: &str, b: &str, limits: Limits) -> DiffResult {
    let mut a = split_lines(a);
    let mut b = split_lines(b);
    let lines_cut = a.len() > limits.max_lines || b.len() > limits.max_lines;
    a.truncate(limits.max_lines);
    b.truncate(limits.max_lines);
    let mut out = DiffResult {
        lines_cut,
        ..DiffResult::default()
    };
    // the common head and tail need no search
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    for l in &a[..head] {
        out.lines.push((Op::Same, l.to_string()));
    }
    let (ma, mb) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    // lines as ids: equal lines compare as integers
    let mut ids = std::collections::HashMap::new();
    let mut id = |l: &str| {
        let n = ids.len() as u32;
        *ids.entry(l.to_string()).or_insert(n)
    };
    let ia: Vec<u32> = ma.iter().map(|l| id(l)).collect();
    let ib: Vec<u32> = mb.iter().map(|l| id(l)).collect();
    match myers(&ia, &ib, limits.max_edits, Instant::now() + limits.deadline) {
        Some(ops) => {
            let (mut x, mut y) = (0, 0);
            for op in ops {
                match op {
                    Op::Same => {
                        out.lines.push((Op::Same, ma[x].to_string()));
                        x += 1;
                        y += 1;
                    }
                    Op::Del => {
                        out.lines.push((Op::Del, ma[x].to_string()));
                        x += 1;
                    }
                    Op::Ins => {
                        out.lines.push((Op::Ins, mb[y].to_string()));
                        y += 1;
                    }
                }
            }
        }
        None => {
            out.gave_up = true;
            out.lines
                .extend(ma.iter().map(|l| (Op::Del, l.to_string())));
            out.lines
                .extend(mb.iter().map(|l| (Op::Ins, l.to_string())));
        }
    }
    for l in &a[a.len() - tail..] {
        out.lines.push((Op::Same, l.to_string()));
    }
    out.added = out.lines.iter().filter(|l| l.0 == Op::Ins).count();
    out.removed = out.lines.iter().filter(|l| l.0 == Op::Del).count();
    out
}

/// Shortest edit script from `a` to `b` (Myers' O((N+M)·D) greedy search
/// with its trace kept for the walk back), or `None` past `max_edits`
/// edits or `deadline`.
fn myers(a: &[u32], b: &[u32], max_edits: usize, deadline: Instant) -> Option<Vec<Op>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    let off = max as isize + 1;
    let mut v = vec![0isize; 2 * max + 3];
    // trace[d] = v for k in -d-1 ..= d+1 before step d
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = None;
    'search: for d in 0..=max as isize {
        if d as usize > max_edits || Instant::now() > deadline {
            return None;
        }
        trace.push(v[(off - d - 1) as usize..=(off + d + 1) as usize].to_vec());
        let mut k = -d;
        while k <= d {
            let i = (off + k) as usize;
            let mut x = if k == -d || (k != d && v[i - 1] < v[i + 1]) {
                v[i + 1]
            } else {
                v[i - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[i] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'search;
            }
            k += 2;
        }
    }
    let found = found?;
    // walk back from (n, m)
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..=found).rev() {
        let t = &trace[d as usize];
        let at = |k: isize| t[(k + d + 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = at(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            ops.push(Op::Same);
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            ops.push(if x == prev_x { Op::Ins } else { Op::Del });
        }
        x = prev_x;
        y = prev_y;
    }
    ops.reverse();
    Some(ops)
}

#[derive(Debug, Default)]
pub enum Phase {
    #[default]
    Loading,
    Ready(Box<DiffResult>),
}

#[derive(Default)]
pub struct CompareState {
    pub open: bool,
    /// Tells this comparison's result from older ones (unique per process).
    pub generation: u64,
    /// First (older, `-`) and second (`+`) block.
    pub a: Side,
    pub b: Side,
    /// How the first block was picked ("previous run of the same command").
    pub baseline: Option<String>,
    pub phase: Phase,
    /// Diff rows scrolled past the top.
    pub scroll: usize,
    /// Rows the body showed last frame.
    pub view_rows: usize,
}

impl CompareState {
    /// Open on `a` and `b` and diff them on a worker; `deliver` gets the
    /// result with this comparison's generation.
    pub fn start(
        &mut self,
        a: Side,
        b: Side,
        baseline: Option<String>,
        deliver: impl FnOnce(u64, DiffResult) + Send + 'static,
    ) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        let (ta, tb) = (a.output.text.clone(), b.output.text.clone());
        *self = CompareState {
            open: true,
            generation,
            a,
            b,
            baseline,
            phase: Phase::Loading,
            ..CompareState::default()
        };
        let spawned = std::thread::Builder::new()
            .name("compare-diff".into())
            .spawn(move || {
                let started = Instant::now();
                let result = diff(&ta, &tb, Limits::default());
                log::info!(
                    "compare: diff of {} rows took {:?} (gave up: {})",
                    result.lines.len(),
                    started.elapsed(),
                    result.gave_up
                );
                deliver(generation, result)
            });
        if let Err(e) = spawned {
            log::warn!("compare: no worker thread: {e}");
        }
    }

    /// Take the worker's result when it is this comparison's.
    pub fn accept(&mut self, generation: u64, result: DiffResult) -> bool {
        if !self.open || generation != self.generation {
            return false;
        }
        self.phase = Phase::Ready(Box::new(result));
        true
    }

    pub fn close(&mut self) {
        *self = CompareState::default();
    }

    fn rows(&self) -> usize {
        match &self.phase {
            Phase::Ready(r) => r.lines.len(),
            Phase::Loading => 0,
        }
    }

    /// Scroll by `rows` (> 0 = down), clamped.
    pub fn scroll_by(&mut self, rows: isize) {
        let max = self.rows().saturating_sub(self.view_rows.max(1));
        self.scroll = (self.scroll as isize + rows).clamp(0, max as isize) as usize;
    }

    pub fn page(&self) -> isize {
        self.view_rows.saturating_sub(1).max(1) as isize
    }

    /// The result in a sentence: counts, or why it is incomplete.
    pub fn summary(&self) -> String {
        let r = match &self.phase {
            Phase::Loading => return "Comparing…".into(),
            Phase::Ready(r) => r,
        };
        let mut parts = Vec::new();
        let incomplete = self.a.gap().is_some() || self.b.gap().is_some() || r.lines_cut;
        if r.equal() {
            parts.push(if incomplete {
                "No differences in the compared part".to_string()
            } else {
                "No differences".to_string()
            });
        } else {
            parts.push(format!("{} added, {} removed", r.added, r.removed));
        }
        if r.lines_cut {
            parts.push(format!("truncated at {MAX_LINES} lines"));
        }
        if r.gave_up {
            parts.push("too many changes: the changed part is shown whole".into());
        }
        parts.join(" · ")
    }

    /// The comparison as a screen reader reads it.
    pub fn a11y_status(&self) -> String {
        let now = SystemTime::now();
        let side = |label: &str, s: &Side| {
            let mut t = format!("{label}: {}, {}", s.command, s.meta(now));
            if let Some(g) = s.gap() {
                t.push_str(&format!(", {g}"));
            }
            t
        };
        let mut parts = vec![side("Before", &self.a), side("After", &self.b)];
        if let Some(b) = &self.baseline {
            parts.push(b.clone());
        }
        parts.push(self.summary());
        if let Phase::Ready(r) = &self.phase {
            for (op, text) in r.lines.iter().filter(|l| l.0 != Op::Same).take(50) {
                let mark = if *op == Op::Ins { "added" } else { "removed" };
                let text = if text.is_empty() {
                    "empty line"
                } else {
                    text.as_str()
                };
                parts.push(format!("{mark}: {text}"));
            }
        }
        parts.join(". ")
    }

    /// One `dump` line.
    pub fn dump(&self) -> String {
        let phase = match &self.phase {
            Phase::Loading => "loading".to_string(),
            Phase::Ready(r) => format!(
                "ready rows={} added={} removed={} gave_up={} lines_cut={}",
                r.lines.len(),
                r.added,
                r.removed,
                r.gave_up,
                r.lines_cut
            ),
        };
        format!(
            "compare={} blocks=({}, {}) phase={phase} summary={:?} gaps=({:?}, {:?}) baseline={:?} scroll={}",
            self.open,
            self.a.block,
            self.b.block,
            self.summary(),
            self.a.gap(),
            self.b.gap(),
            self.baseline,
            self.scroll
        )
    }
}

/// Click targets of the last frame.
#[derive(Default, Clone, Copy)]
pub struct CompareHits {
    pub card: [f32; 4],
    pub close: [f32; 4],
}

const MODAL_W: f32 = 960.0;
const PAD_X: f32 = 24.0;
const HEADER_H: f32 = 56.0;
const SIDE_H: f32 = 44.0;
const FOOTER_H: f32 = 40.0;
const ROW_H: f32 = 18.0;

fn label(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    family: &'static str,
    weight: Weight,
) -> Buffer {
    let mut buffer = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    buffer.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(family)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buffer
}

/// `s` cut to `n` chars with `…`.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push('…');
        t
    }
}

pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &mut CompareState,
    w: f32,
    h: f32,
) -> CompareHits {
    layer.rect([0.0, 0.0], [w, h], Color::BLACK.with_alpha(0.55), 0.0);
    let modal_w = MODAL_W.min(w - 32.0);
    let modal_h = (h - 72.0).clamp(HEADER_H + SIDE_H * 2.0 + FOOTER_H + 60.0, 760.0);
    let x = ((w - modal_w) / 2.0).round();
    let y = ((h - modal_h) / 2.0).round();
    layer.shadow(
        [x, y + 10.0],
        [modal_w, modal_h],
        Color::BLACK.with_alpha(0.62),
        8.0,
        20.0,
    );
    layer.bordered(
        [x, y],
        [modal_w, modal_h],
        theme.surface,
        Color::WHITE.with_alpha(0.12),
        8.0,
        1.0,
    );

    // ── header: title, close ──
    layer.text(
        label(engine, "Compare outputs", 17.0, FONT_SANS, Weight::SEMIBOLD),
        x + PAD_X,
        y + 17.0,
        theme.text_bright,
    );
    if let Some(b) = &state.baseline {
        let tw = engine.measure("Compare outputs", 17.0, FONT_SANS, Weight::SEMIBOLD);
        layer.text(
            label(engine, b, 12.0, FONT_SANS, Weight::NORMAL),
            x + PAD_X + tw + 12.0,
            y + 21.0,
            theme.text_muted,
        );
    }
    let close = [x + modal_w - 16.0 - 28.0, y + 14.0, 28.0, 28.0];
    layer.rect(
        [close[0], close[1]],
        [close[2], close[3]],
        Color::WHITE.with_alpha(0.06),
        4.0,
    );
    super::close_icon(
        layer,
        [close[0] + close[2] / 2.0, close[1] + close[3] / 2.0],
        4.0,
        theme.text_muted,
    );
    layer.hline(x + 1.0, y + HEADER_H, modal_w - 2.0, 1.0, theme.border);

    // ── the two blocks: mark, command, metadata (and what is missing) ──
    let now = SystemTime::now();
    let bounds_all = TextBounds {
        left: x as i32,
        top: y as i32,
        right: (x + modal_w - PAD_X) as i32,
        bottom: (y + modal_h) as i32,
    };
    for (i, (side, mark, color)) in [(&state.a, "−", theme.error), (&state.b, "+", theme.success)]
        .into_iter()
        .enumerate()
    {
        let sy = y + HEADER_H + 1.0 + i as f32 * SIDE_H;
        layer.text(
            label(engine, mark, 14.0, mono_family(), Weight::BOLD),
            x + PAD_X,
            sy + 6.0,
            color,
        );
        let cmd = if side.command.is_empty() {
            "(no command)".to_string()
        } else {
            cut(&side.command, 160)
        };
        layer.text_clipped(
            label(engine, &cmd, 13.0, mono_family(), Weight::NORMAL),
            x + PAD_X + 20.0,
            sy + 6.0,
            theme.text_bright,
            bounds_all,
        );
        let mut meta = side.meta(now);
        let gap = side.gap();
        if let Some(g) = &gap {
            meta.push_str(" · ");
            meta.push_str(g);
        }
        layer.text_clipped(
            label(engine, &meta, 11.5, FONT_SANS, Weight::NORMAL),
            x + PAD_X + 20.0,
            sy + 24.0,
            if gap.is_some() {
                theme.warning
            } else {
                theme.text_dim
            },
            bounds_all,
        );
    }
    let body_top = y + HEADER_H + 1.0 + SIDE_H * 2.0;
    layer.hline(x + 1.0, body_top, modal_w - 2.0, 1.0, theme.border);

    // ── body: the diff rows on screen ──
    let body_bottom = y + modal_h - FOOTER_H;
    let rows_fit = (((body_bottom - body_top - 8.0) / ROW_H).floor() as usize).max(1);
    state.view_rows = rows_fit;
    state.scroll_by(0);
    let bounds = TextBounds {
        left: (x + 1.0) as i32,
        top: body_top as i32,
        right: (x + modal_w - 12.0) as i32,
        bottom: body_bottom as i32,
    };
    let center = |engine: &mut TextEngine, layer: &mut Layer, text: &str, color: Color| {
        let tw = engine.measure(text, 13.0, FONT_SANS, Weight::NORMAL);
        layer.text(
            label(engine, text, 13.0, FONT_SANS, Weight::NORMAL),
            x + (modal_w - tw) / 2.0,
            body_top + (body_bottom - body_top) / 2.0 - 9.0,
            color,
        );
    };
    match &state.phase {
        Phase::Loading => center(engine, layer, "Comparing…", theme.text_muted),
        Phase::Ready(r) if r.equal() => center(engine, layer, &state.summary(), theme.text_dim),
        Phase::Ready(r) => {
            let digits = r.lines.len().to_string().len();
            for (row, (op, text)) in r.lines.iter().enumerate().skip(state.scroll).take(rows_fit) {
                let ry = body_top + 4.0 + (row - state.scroll) as f32 * ROW_H;
                let (mark, tint, fg) = match op {
                    Op::Same => (" ", None, theme.text_ui),
                    Op::Del => ("-", Some(theme.error), theme.text_bright),
                    Op::Ins => ("+", Some(theme.success), theme.text_bright),
                };
                if let Some(t) = tint {
                    layer.rect(
                        [x + 1.0, ry],
                        [modal_w - 2.0, ROW_H],
                        t.with_alpha(0.14),
                        0.0,
                    );
                    layer.rect([x + 1.0, ry], [3.0, ROW_H], t, 0.0);
                }
                let num = format!("{:>digits$}", row + 1);
                layer.text(
                    label(engine, &num, 11.5, mono_family(), Weight::NORMAL),
                    x + 12.0,
                    ry + 1.5,
                    theme.text_muted,
                );
                let tx = x + 12.0 + (digits as f32 + 1.0) * 7.5;
                layer.text(
                    label(engine, mark, 12.5, mono_family(), Weight::NORMAL),
                    tx,
                    ry + 1.0,
                    tint.unwrap_or(theme.text_muted),
                );
                if !text.is_empty() {
                    layer.text_clipped(
                        label(
                            engine,
                            &cut(text, ROW_CHARS),
                            12.5,
                            mono_family(),
                            Weight::NORMAL,
                        ),
                        tx + 16.0,
                        ry + 1.0,
                        fg,
                        bounds,
                    );
                }
            }
            // scrollbar when the rows overflow
            if r.lines.len() > rows_fit {
                let view_h = body_bottom - body_top;
                let track = view_h - 16.0;
                let thumb = (track * rows_fit as f32 / r.lines.len() as f32).max(24.0);
                let max = (r.lines.len() - rows_fit) as f32;
                let ty = body_top + 8.0 + (track - thumb) * (state.scroll as f32 / max);
                layer.rect(
                    [x + modal_w - 8.0, ty],
                    [3.0, thumb],
                    Color::WHITE.with_alpha(0.18),
                    1.5,
                );
            }
        }
    }

    // ── footer: the result, keys ──
    layer.hline(x, body_bottom, modal_w, 1.0, theme.border);
    let incomplete = state.a.gap().is_some()
        || state.b.gap().is_some()
        || matches!(&state.phase, Phase::Ready(r) if r.lines_cut || r.gave_up);
    layer.text(
        label(engine, &state.summary(), 12.0, FONT_SANS, Weight::MEDIUM),
        x + PAD_X,
        body_bottom + 12.0,
        if incomplete {
            theme.warning
        } else {
            theme.text_ui
        },
    );
    let keys = "↑↓ PgUp PgDn scroll · Esc close";
    let kw = engine.measure(keys, 11.5, FONT_SANS, Weight::NORMAL);
    layer.text(
        label(engine, keys, 11.5, FONT_SANS, Weight::NORMAL),
        x + modal_w - PAD_X - kw,
        body_bottom + 12.5,
        theme.text_muted,
    );

    CompareHits {
        card: [x, y, modal_w, modal_h],
        close,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(r: &DiffResult) -> String {
        r.lines
            .iter()
            .map(|(op, t)| {
                let m = match op {
                    Op::Same => ' ',
                    Op::Del => '-',
                    Op::Ins => '+',
                };
                format!("{m}{t}")
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn added_removed_unchanged_and_empty_lines() {
        let r = diff("a\nb\n\nc\n", "a\n\nc\nd\n", Limits::default());
        assert_eq!(show(&r), " a|-b| | c|+d");
        assert_eq!((r.added, r.removed), (1, 1));
        // an empty line that appears or goes is a change of its own
        let r = diff("x\ny\n", "x\n\ny\n", Limits::default());
        assert_eq!(show(&r), " x|+| y");
        assert_eq!((r.added, r.removed), (1, 0));
    }

    #[test]
    fn equal_output_is_no_differences() {
        let r = diff("same\nlines\n", "same\nlines\n", Limits::default());
        assert!(r.equal() && !r.gave_up);
        let r = diff("", "", Limits::default());
        assert!(r.equal() && r.lines.is_empty());
    }

    #[test]
    fn final_newline_and_unicode() {
        // a missing final newline does not count as a change
        assert!(diff("a\nb", "a\nb\n", Limits::default()).equal());
        let r = diff(
            "日本語 ok\ncafé\n",
            "日本語 ok\ncafe\u{301}\n",
            Limits::default(),
        );
        // composed vs decomposed text is different output
        assert_eq!(show(&r), " 日本語 ok|-café|+cafe\u{301}");
        let r = diff("", "only\n", Limits::default());
        assert_eq!(show(&r), "+only");
        let r = diff("gone\n", "", Limits::default());
        assert_eq!(show(&r), "-gone");
    }

    #[test]
    fn interleaved_edits_find_the_shortest_script() {
        let a = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let b = "1\nX\n3\n4\nY\n6\n8\nZ\n";
        let r = diff(a, b, Limits::default());
        assert_eq!((r.added, r.removed), (3, 3), "{}", show(&r));
        // applying the script to a gives b
        let rebuilt: Vec<&str> = r
            .lines
            .iter()
            .filter(|l| l.0 != Op::Del)
            .map(|l| l.1.as_str())
            .collect();
        assert_eq!(rebuilt.join("\n") + "\n", b);
        let orig: Vec<&str> = r
            .lines
            .iter()
            .filter(|l| l.0 != Op::Ins)
            .map(|l| l.1.as_str())
            .collect();
        assert_eq!(orig.join("\n") + "\n", a);
    }

    #[test]
    fn edit_and_line_bounds_are_reported() {
        let a: String = (0..300).map(|i| format!("a{i}\n")).collect();
        let b: String = (0..300).map(|i| format!("b{i}\n")).collect();
        let limits = Limits {
            max_edits: 50,
            ..Limits::default()
        };
        let r = diff(&a, &b, limits);
        assert!(r.gave_up, "past the edit bound the region shows whole");
        assert_eq!((r.added, r.removed), (300, 300));
        let limits = Limits {
            max_lines: 10,
            ..Limits::default()
        };
        let r = diff(&a, &a, limits);
        assert!(r.lines_cut && r.lines.len() == 10 && r.equal());
    }

    #[test]
    fn adversarial_size_stays_bounded_in_time() {
        // every line different on both sides, at the line bound: the diff
        // gives up at the edit bound instead of searching N·M
        let a: String = (0..MAX_LINES * 2).map(|i| format!("left {i}\n")).collect();
        let b: String = (0..MAX_LINES * 2).map(|i| format!("right {i}\n")).collect();
        let t = Instant::now();
        let r = diff(&a, &b, Limits::default());
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        assert!(r.gave_up && r.lines_cut);
        assert_eq!(r.lines.len(), MAX_LINES * 2);
    }

    #[test]
    fn stale_results_are_dropped() {
        let mut s = CompareState {
            open: true,
            generation: 7,
            ..Default::default()
        };
        assert!(
            !s.accept(6, DiffResult::default()),
            "an older comparison's result"
        );
        assert!(matches!(s.phase, Phase::Loading));
        assert!(s.accept(7, DiffResult::default()));
        s.close();
        assert!(!s.accept(7, DiffResult::default()), "closed");
    }

    #[test]
    fn gaps_are_labeled() {
        let mut s = Side::default();
        assert_eq!(s.gap(), None);
        s.output.total_rows = 10;
        s.output.trimmed_rows = 10;
        assert_eq!(
            s.gap().unwrap(),
            "output unavailable (trimmed from scrollback)"
        );
        s.output.trimmed_rows = 4;
        s.output.truncated = true;
        assert_eq!(
            s.gap().unwrap(),
            "first 4 rows trimmed from scrollback, cut at 20000 lines / 4 MB"
        );
        let mut st = CompareState {
            a: s,
            phase: Phase::Ready(Box::default()),
            ..Default::default()
        };
        assert_eq!(st.summary(), "No differences in the compared part");
        st.a = Side::default();
        assert_eq!(st.summary(), "No differences");
    }
}
