//! File explorer: a left panel with the active pane's project as a lazily
//! expanded tree (`shift_core::fs_tree`), tinted by git status from the
//! review model. Click a folder to expand it, a file to open it in the
//! viewer; ⌥-click inserts the path into the command line.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use shift_core::fs_tree::{self, Entry};
use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

pub const EXPLORER_W: f32 = 260.0;
pub const HEADER_H: f32 = 38.0;
pub const ROW_H: f32 = 24.0;
const INDENT: f32 = 14.0;

#[derive(Default)]
pub struct ExplorerState {
    pub open: bool,
    /// Keyboard focus is in the tree (arrows, Enter, Esc).
    pub focused: bool,
    pub root: Option<PathBuf>,
    last_cwd: Option<PathBuf>,
    repo: Option<git2::Repository>,
    pub expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    pub selected: Option<PathBuf>,
    /// Vertical scroll (logical px, ≥ 0).
    pub scroll: f32,
    pub view_h: f32,
    /// Visible tree of the last frame, in order.
    pub rows: Vec<Row>,
    /// Git status per repository-relative path, and folders holding
    /// changes (from the review model).
    statuses: HashMap<String, char>,
    dirty_dirs: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExplorerHit {
    Row(usize),
    Refresh,
    Collapse,
}

impl ExplorerState {
    /// Point the tree at the project containing `cwd`; a new root resets it.
    /// True when the root changed.
    pub fn follow(&mut self, cwd: &Path) -> bool {
        if self.last_cwd.as_deref() == Some(cwd) {
            return false;
        }
        self.last_cwd = Some(cwd.to_path_buf());
        let root = fs_tree::root_for(cwd);
        if self.root.as_deref() == Some(root.as_path()) {
            return false;
        }
        self.repo = git2::Repository::discover(&root).ok();
        self.root = Some(root);
        self.expanded.clear();
        self.children.clear();
        self.selected = None;
        self.scroll = 0.0;
        true
    }

    pub fn set_statuses(&mut self, statuses: HashMap<String, char>, dirty_dirs: HashSet<String>) {
        self.statuses = statuses;
        self.dirty_dirs = dirty_dirs;
    }

    /// Re-read every listing (after a command, a git write, ↻).
    pub fn refresh(&mut self) {
        self.children.clear();
        if let Some(root) = &self.root {
            self.repo = git2::Repository::discover(root).ok();
        }
    }

    fn listing(&mut self, dir: &Path) -> &Vec<Entry> {
        if !self.children.contains_key(dir) {
            let list = fs_tree::list(dir, self.repo.as_ref());
            self.children.insert(dir.to_path_buf(), list);
        }
        &self.children[dir]
    }

    /// Flatten the expanded tree into `rows`.
    pub fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        if let Some(root) = self.root.clone() {
            self.walk(&root, 0, &mut rows);
        }
        self.rows = rows;
    }

    fn walk(&mut self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        let entries = self.listing(dir).clone();
        for e in entries {
            let open = e.is_dir && self.expanded.contains(&e.path);
            rows.push(Row {
                path: e.path.clone(),
                name: e.name,
                depth,
                is_dir: e.is_dir,
            });
            if open {
                self.walk(&e.path, depth + 1, rows);
            }
        }
    }

    pub fn toggle(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        self.scroll = 0.0;
    }

    pub fn scroll_by(&mut self, px: f32) {
        let max = (self.rows.len() as f32 * ROW_H - self.view_h + 8.0).max(0.0);
        self.scroll = (self.scroll + px).clamp(0.0, max);
    }

    fn selected_index(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        self.rows.iter().position(|r| &r.path == sel)
    }

    /// Arrow-key move; keeps the selection on screen.
    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let next = match self.selected_index() {
            Some(i) => (i as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize,
            None => 0,
        };
        self.selected = Some(self.rows[next].path.clone());
        self.reveal(next);
    }

    fn reveal(&mut self, i: usize) {
        let top = i as f32 * ROW_H;
        if top < self.scroll {
            self.scroll = top;
        } else if top + ROW_H > self.scroll + self.view_h {
            self.scroll = top + ROW_H - self.view_h;
        }
    }

    /// ← on a selection: collapse it, or jump to its parent folder.
    pub fn left(&mut self) {
        let Some(i) = self.selected_index() else {
            return;
        };
        let row = self.rows[i].clone();
        if row.is_dir && self.expanded.remove(&row.path) {
            return;
        }
        if let Some(parent) = row.path.parent()
            && let Some(p) = self.rows.iter().position(|r| r.path == parent)
        {
            self.selected = Some(parent.to_path_buf());
            self.reveal(p);
        }
    }

    /// → on a folder: expand it.
    pub fn right(&mut self) {
        if let Some(i) = self.selected_index()
            && self.rows[i].is_dir
        {
            self.expanded.insert(self.rows[i].path.clone());
        }
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected_index()?)
    }
}

/// Chevrons come from the mono face (Inter has no ▸ ▾), like block headers.
fn mono(engine: &mut TextEngine, text: &str, size: f32) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    b.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(mono_family())),
        Shaping::Advanced,
        None,
    );
    b
}

fn label(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    b.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    b
}

/// Git status color (meaning, not decoration).
pub fn status_color(theme: &Theme, status: char) -> Color {
    match status {
        'A' | '?' => theme.success,
        'D' | 'U' => theme.error,
        _ => theme.warning,
    }
}

/// Draw the panel into `rect` = [x, y, w, h]; returns click targets.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &mut ExplorerState,
    rect: [f32; 4],
) -> Vec<([f32; 4], ExplorerHit)> {
    let [x, y, w, h] = rect;
    let mut hits = Vec::new();
    layer.rect([x, y], [w, h], theme.bg_chrome, 0.0);
    layer.rect([x + w - 1.0, y], [1.0, h], theme.border, 0.0);

    // ── header: project name, collapse-all, refresh ──
    let title = state
        .root
        .as_ref()
        .and_then(|r| r.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "No folder".into());
    layer.text(
        label(engine, &title.to_uppercase(), 11.0, Weight::SEMIBOLD),
        x + 14.0,
        y + 12.0,
        theme.text_dim,
    );
    let refresh = [x + w - 34.0, y + 7.0, 24.0, 24.0];
    let collapse = [refresh[0] - 26.0, y + 7.0, 24.0, 24.0];
    for (r, glyph, hit) in [
        (collapse, "⊟", ExplorerHit::Collapse),
        (refresh, "↻", ExplorerHit::Refresh),
    ] {
        layer.text(
            label(engine, glyph, 13.0, Weight::NORMAL),
            r[0] + 6.0,
            r[1] + 3.0,
            theme.text_muted,
        );
        hits.push((r, hit));
    }
    layer.hline(x, y + HEADER_H, w - 1.0, 1.0, theme.border);

    // ── tree: only rows on screen are shaped ──
    state.rebuild_rows();
    let body_top = y + HEADER_H + 1.0;
    let body_h = h - HEADER_H - 1.0;
    state.view_h = body_h;
    state.scroll_by(0.0);
    let bounds = TextBounds {
        left: x as i32,
        top: body_top as i32,
        right: (x + w - 8.0) as i32,
        bottom: (y + h) as i32,
    };
    let root = state.root.clone().unwrap_or_default();
    let first = (state.scroll / ROW_H).floor() as usize;
    let last = ((state.scroll + body_h) / ROW_H).ceil() as usize + 1;
    for (i, row) in state.rows.iter().enumerate().take(last).skip(first) {
        let ry = body_top + 4.0 + i as f32 * ROW_H - state.scroll;
        let row_rect = [x, ry, w - 1.0, ROW_H];
        if state.selected.as_ref() == Some(&row.path) {
            let wash = if state.focused { 0.22 } else { 0.12 };
            // quads can't be clipped: keep the wash inside the body
            let top = ry.max(body_top);
            let bottom = (ry + ROW_H).min(y + h);
            if bottom > top {
                layer.rect(
                    [x, top],
                    [w - 1.0, bottom - top],
                    theme.accent.with_alpha(wash),
                    0.0,
                );
            }
        }
        let ix = x + 12.0 + row.depth as f32 * INDENT;
        let key = fs_tree::rel_key(&root, &row.path).unwrap_or_default();
        let status = state.statuses.get(&key).copied();
        let dirty = row.is_dir && state.dirty_dirs.contains(&key);
        if row.is_dir {
            let chevron = if state.expanded.contains(&row.path) {
                "▾"
            } else {
                "▸"
            };
            layer.text_clipped(
                mono(engine, chevron, 11.0),
                ix,
                ry + 6.0,
                theme.text_muted,
                bounds,
            );
        }
        let name_color = match status {
            Some(s) => status_color(theme, s),
            None if row.is_dir => theme.text_ui,
            None => theme.text_dim,
        };
        layer.text_clipped(
            label(engine, &row.name, 12.5, Weight::NORMAL),
            ix + 14.0,
            ry + 4.0,
            name_color,
            bounds,
        );
        if let Some(s) = status {
            let letter = if s == '?' { 'U' } else { s };
            layer.text_clipped(
                label(engine, &letter.to_string(), 11.0, Weight::SEMIBOLD),
                x + w - 24.0,
                ry + 5.0,
                status_color(theme, s),
                bounds,
            );
        } else if dirty {
            let cy = ry + ROW_H / 2.0;
            if cy - 3.0 > body_top {
                layer.circle([x + w - 20.0, cy], 5.0, theme.warning.with_alpha(0.7));
            }
        }
        if ry + ROW_H > body_top {
            let top = ry.max(body_top);
            hits.push((
                [row_rect[0], top, row_rect[2], ry + ROW_H - top],
                ExplorerHit::Row(i),
            ));
        }
    }
    if state.rows.is_empty() {
        layer.text(
            label(engine, "Empty folder", 12.0, Weight::NORMAL),
            x + 14.0,
            body_top + 12.0,
            theme.text_muted,
        );
    }
    hits
}
