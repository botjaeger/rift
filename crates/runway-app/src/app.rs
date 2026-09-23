use std::sync::Arc;
use std::time::{Duration, Instant};

use runway_core::git::{GitInfo, GitWorker};
use runway_core::{BlockState, Config, Keymap, Theme};
use runway_pty::{GridSize, PaneId, Session, SessionEvent, SpawnOptions};
use runway_renderer::glyphon::{TextArea, TextBounds};
use runway_renderer::{Gpu, QuadPipeline, Scene, TextEngine, to_wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowAttributes, WindowId};

use runway_pty::alacritty_terminal::index::{
    Column as ACol, Line as ALine, Point as APoint, Side as ASide,
};
use runway_pty::alacritty_terminal::selection::{Selection, SelectionType};

use crate::input::{self, AppAction, EditorKey};
use crate::pane::{Axis, HandleRect, NodePath, PaneRect, focus_neighbor};
use crate::state::{ClickTarget, Hit, PaneState, TabState};
use crate::views::completion as completion_view;
use crate::views::completion::CompletionUi;
use crate::views::context_menu::{self, ContextMenuState, MenuAction, MenuItem};
use crate::views::editor::{self, EDITOR_H};
use crate::views::palette::{self, MatchedItem, PaletteAction, PaletteState};
use crate::views::search::{self, SearchState};
use crate::views::settings::{self, SettingsState};
use crate::views::tabbar::{self, TAB_BAR_H};
use crate::views::terminal::{self, GridLayout};

/// Cross-thread wakeups.
#[derive(Debug)]
pub enum UserEvent {
    PtyWakeup(PaneId),
    PtyTitle(PaneId, String),
    PtyExit(PaneId),
    PtyBell(PaneId),
    /// OSC 52 copy from the program in the pane.
    PtyClipboard(String),
    /// OSC 4/10/11/12 color query to answer from the theme.
    PtyColorRequest(PaneId, usize, runway_pty::ColorReply),
    /// The system-wide `global_hotkey` was pressed.
    GlobalHotkey,
    /// A native menu item was chosen (its id).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Menu(String),
    GitStatus(PaneId, Option<GitInfo>),
    ConfigReloaded,
    HistoryLoaded(Vec<String>),
    ShellEnvLoaded(Arc<runway_core::shell_env::ShellEnv>),
}

/// Spec metrics (logical px).
const SIDE_PAD: f32 = 8.0;
const TOP_PAD: f32 = 14.0;
const DEFAULT_FONT_SIZE: f32 = 13.0;
const BLINK_INTERVAL: Duration = Duration::from_millis(550);
const BELL_FLASH: Duration = Duration::from_millis(120);
/// Pane chrome (design 02).
const PANE_RADIUS: f32 = 9.0;
const PANE_PAD: f32 = 12.0;

struct DragState {
    path: NodePath,
    axis: Axis,
    /// Area of the split node being resized.
    area: [f32; 4],
}

/// Armed on left-press over terminal content; becomes a live selection once
/// the pointer moves past a small threshold.
struct PendingSelection {
    pane: PaneId,
    point: APoint,
    side: ASide,
    origin: (f32, f32),
    started: bool,
    /// ⌥-drag: rectangular (column) selection.
    block: bool,
}

const DOUBLE_CLICK_MS: u128 = 400;
const DRAG_THRESHOLD: f32 = 3.0;

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    quads: Option<QuadPipeline>,
    /// Theme `background_image` (Warp-style), drawn under every layer.
    bg_image: Option<runway_renderer::ImagePipeline>,
    bg_image_opacity: f32,
    text: Option<TextEngine>,
    git: Option<GitWorker>,
    tabs: Vec<TabState>,
    active: usize,
    next_pane: PaneId,
    scene: Scene,
    theme: Theme,
    cell_w: f32,
    modifiers: ModifiersState,
    hits: Vec<Hit>,
    mouse: (f32, f32),
    any_running: bool,
    editor_active: bool,
    blink_epoch: Instant,
    drag: Option<DragState>,
    palette: PaletteState,
    palette_results: Vec<MatchedItem>,
    palette_row_hits: Vec<([f32; 4], usize)>,
    completion: CompletionUi,
    editor_anchor: (f32, f32),
    /// Soft-wrap columns of the active pane's editor (last frame).
    editor_cols: usize,
    /// Find-bar toggle chips: [match case, regex] (last frame).
    search_chips: Vec<[f32; 4]>,
    config: Config,
    keymap: Keymap,
    settings: SettingsState,
    slider_drag: Option<(&'static str, [f32; 4])>,
    search: SearchState,
    clipboard: Option<arboard::Clipboard>,
    pending_sel: Option<PendingSelection>,
    last_click: Option<(Instant, (f32, f32), u8)>,
    context_menu: ContextMenuState,
    context_menu_rects: Vec<[f32; 4]>,
    shell_history: Vec<String>,
    workflows: Vec<runway_core::workflows::Workflow>,
    font_size: f32,
    line_h: f32,
    scroll_accum: f32,
    /// cwds of recently closed tabs (⇧⌘T reopens, newest last).
    closed_tabs: Vec<Option<std::path::PathBuf>>,
    _watcher: Option<runway_core::watch::ConfigWatcher>,
    /// `--script` driver (see script.rs).
    script: Option<crate::script::Script>,
    /// Next frame is also rendered offscreen and written to this PNG.
    capture: Option<std::path::PathBuf>,
    /// OS appearance is light (drives `sync_os_theme`).
    os_light: bool,
    /// Window has keyboard focus (completion notifications fire only when it doesn't).
    focused: bool,
    /// Visual bell flash start.
    bell_flash: Option<Instant>,
    /// Newest block id per pane already checked for a completion notification.
    notified: std::collections::HashMap<PaneId, runway_core::BlockId>,
    /// Warp-style launch configurations (~/.config/runway/launch_configs).
    launch_configs: Vec<runway_core::launch::LaunchConfig>,
    /// Commands a launch configuration queued for a pane; sent at its first prompt.
    pending_commands: std::collections::HashMap<PaneId, Vec<String>>,
    /// Running-process close/quit warning awaiting a decision.
    confirm: Option<ConfirmClose>,
    /// [cancel, destructive] button rects of the confirm modal.
    confirm_rects: [[f32; 4]; 2],
    /// Inline tab rename in progress: (tab index, edited title).
    rename: Option<(usize, String)>,
    /// Rename text still fully selected (first keystroke replaces it).
    rename_fresh: bool,
    /// Search overlay edits a per-block output filter instead of find.
    filter_editing: bool,
    /// Aliases/functions/builtins probed from the user's shell (background).
    shell_env: Option<Arc<runway_core::shell_env::ShellEnv>>,
    /// Session file is read/written (off for secondary windows and scripts).
    persist_session: bool,
    /// Tab being dragged in the tab bar: (index, press x).
    tab_drag: Option<(usize, f32)>,
    /// Editor mouse selection in progress (pane).
    editor_drag: Option<PaneId>,
    /// Transient status message (text, shown at).
    toast: Option<(String, Instant)>,
    /// Mouse button held down while reporting to a mouse-tracking app:
    /// (pane, button code, last reported cell).
    mouse_report: Option<(PaneId, u8, (usize, usize))>,
    /// Last cell a button-less motion report was sent for (mode 1003).
    mouse_hover_cell: Option<(PaneId, (usize, usize))>,
    /// IME composition in progress (preedit text, shown at the cursor).
    preedit: String,
    /// Cursor position of the active pane last frame (IME candidate window).
    ime_spot: Option<(f32, f32)>,
    /// Last position handed to the OS input method.
    ime_area_sent: Option<(f32, f32)>,
    /// Pointer shape currently set on the window.
    cursor_icon: winit::window::CursorIcon,
    /// Registered `global_hotkey` (re-registered when the setting changes).
    global_hotkey: Option<crate::hotkey::GlobalHotkey>,
    /// Native menu bar and the keybindings it was built for.
    #[cfg(target_os = "macos")]
    menubar: Option<(
        crate::menubar::MenuBar,
        std::collections::BTreeMap<String, String>,
    )>,
}

/// `YYYY-MM-DD HH:MM` (UTC) for `secs` since the epoch (no date crate).
fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// How long a toast stays up.
const TOAST_TTL: Duration = Duration::from_millis(2600);

/// What a confirmed close/quit acts on.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CloseScope {
    Pane(PaneId),
    Tab(usize),
    OtherTabs(usize),
    Window,
}

struct ConfirmClose {
    scope: CloseScope,
    commands: Vec<String>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>, script: Option<crate::script::Script>) -> Self {
        // scripted runs and secondary windows never touch the user's session
        // file (a script run used to overwrite it)
        let persist_session = (script.is_none()
            || std::env::var_os("RUNWAY_PERSIST_SESSION").is_some())
            && std::env::var_os("RUNWAY_SECONDARY_WINDOW").is_none();
        App {
            proxy,
            window: None,
            gpu: None,
            quads: None,
            bg_image: None,
            bg_image_opacity: 1.0,
            text: None,
            git: None,
            tabs: Vec::new(),
            active: 0,
            next_pane: 1,
            scene: Scene::default(),
            theme: Theme::default(),
            cell_w: DEFAULT_FONT_SIZE * 0.6,
            modifiers: ModifiersState::empty(),
            hits: Vec::new(),
            mouse: (0.0, 0.0),
            any_running: false,
            editor_active: false,
            blink_epoch: Instant::now(),
            drag: None,
            palette: PaletteState::default(),
            palette_results: Vec::new(),
            palette_row_hits: Vec::new(),
            completion: CompletionUi::default(),
            editor_anchor: (0.0, 0.0),
            editor_cols: 80,
            search_chips: Vec::new(),
            config: Config::default(),
            keymap: Keymap::from_overrides(&Default::default()),
            settings: SettingsState::default(),
            slider_drag: None,
            search: SearchState::default(),
            clipboard: None,
            pending_sel: None,
            last_click: None,
            context_menu: ContextMenuState::default(),
            context_menu_rects: Vec::new(),
            shell_history: Vec::new(),
            workflows: Vec::new(),
            font_size: DEFAULT_FONT_SIZE,
            line_h: DEFAULT_FONT_SIZE * 1.65,
            scroll_accum: 0.0,
            closed_tabs: Vec::new(),
            _watcher: None,
            script,
            capture: None,
            os_light: false,
            focused: true,
            bell_flash: None,
            notified: std::collections::HashMap::new(),
            launch_configs: Vec::new(),
            pending_commands: std::collections::HashMap::new(),
            confirm: None,
            confirm_rects: [[0.0; 4]; 2],
            rename: None,
            rename_fresh: false,
            filter_editing: false,
            shell_env: None,
            persist_session,
            tab_drag: None,
            editor_drag: None,
            toast: None,
            mouse_report: None,
            mouse_hover_cell: None,
            preedit: String::new(),
            ime_spot: None,
            ime_area_sent: None,
            cursor_icon: winit::window::CursorIcon::Default,
            global_hotkey: None,
            #[cfg(target_os = "macos")]
            menubar: None,
        }
    }

    /// Re-apply everything derived from Config (theme, metrics, keymap).
    fn apply_config(&mut self) {
        self.theme = self.config.resolve_theme_for(self.os_light);
        let opacity = self.config.opacity();
        if opacity < 1.0 {
            // only the window grounds go translucent; text, borders and
            // overlays keep their contrast (Warp "window opacity")
            self.theme.bg = self.theme.bg.with_alpha(opacity);
            self.theme.bg_chrome = self.theme.bg_chrome.with_alpha(opacity);
            self.theme.bg_gutter = self.theme.bg_gutter.with_alpha(opacity);
        }
        self.font_size = self.config.font_size.clamp(8.0, 28.0);
        self.line_h = self.font_size * self.config.line_height.clamp(1.0, 2.2);
        self.keymap = Keymap::from_overrides(&self.config.keybindings);
        self.apply_global_hotkey();
        // menu shortcut labels follow the keymap
        #[cfg(target_os = "macos")]
        if self.window.is_some()
            && self
                .menubar
                .as_ref()
                .is_none_or(|(_, keys)| *keys != self.config.keybindings)
        {
            self.menubar =
                crate::menubar::install(&self.keymap).map(|m| (m, self.config.keybindings.clone()));
        }
        if let Some(engine) = &mut self.text {
            let used = engine.set_mono_family(&self.config.font_family);
            if !used.eq_ignore_ascii_case(self.config.font_family.trim()) {
                log::warn!(
                    "font_family {:?} not installed, using {used}",
                    self.config.font_family
                );
            }
            self.cell_w = engine.mono_cell_width(self.font_size);
        }
        self.apply_background_image();
        let vim = self.config.vim_mode;
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.editor.set_vim_enabled(vim);
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// (Re)register `global_hotkey` when it changed. Scripted runs and
    /// secondary windows never grab a system-wide key.
    fn apply_global_hotkey(&mut self) {
        let spec = self.config.global_hotkey.trim().to_string();
        if self.global_hotkey.as_ref().map(|h| h.spec()) == Some(spec.as_str())
            || (spec.is_empty() && self.global_hotkey.is_none())
        {
            return;
        }
        self.global_hotkey = None; // drop unregisters the old key
        if spec.is_empty() || !self.persist_session || self.window.is_none() {
            return;
        }
        let proxy = self.proxy.clone();
        match crate::hotkey::register(&spec, move || {
            let _ = proxy.send_event(UserEvent::GlobalHotkey);
        }) {
            Ok(h) => {
                log::info!("global hotkey {spec} registered");
                self.global_hotkey = Some(h);
            }
            Err(e) => {
                log::warn!("{e}");
                self.toast(e);
            }
        }
    }

    /// Global hotkey: bring Runway forward, or hide it when it is already
    /// the focused app (Warp hotkey window behavior).
    fn toggle_visibility(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        #[cfg(target_os = "macos")]
        {
            if crate::hotkey::app_active() && self.focused {
                crate::hotkey::set_app_shown(false);
            } else {
                crate::hotkey::set_app_shown(true);
                window.set_minimized(false);
                window.focus_window();
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if self.focused && window.is_visible() != Some(false) {
                window.set_minimized(true);
            } else {
                window.set_minimized(false);
                window.set_visible(true);
                window.focus_window();
            }
        }
    }

    /// Load (or drop) the theme's background image into the GPU pipeline.
    fn apply_background_image(&mut self) {
        let (Some(gpu), Some(pipe)) = (&self.gpu, &mut self.bg_image) else {
            return;
        };
        match &self.theme.background_image {
            Some(img) => {
                let mut path = std::path::PathBuf::from(&img.path);
                if path.is_relative()
                    && let Some(dir) = runway_core::config::themes_dir()
                {
                    path = dir.join(path);
                }
                self.bg_image_opacity = img.opacity.clamp(0.0, 1.0);
                if let Err(e) = pipe.load(&gpu.device, &gpu.queue, &path) {
                    log::warn!("background_image {}: {e}", path.display());
                }
            }
            None => pipe.clear(),
        }
    }

    fn save_config(&self) {
        if let Err(e) = self.config.save() {
            log::warn!("config save failed: {e}");
        }
    }

    /// Map a mouse position to a grid Point + cell Side using the pane's
    /// frame row cache. Header rows clamp to the nearest grid row.
    fn grid_point_at(pane: &PaneState, mx: f32, my: f32) -> Option<(APoint, ASide)> {
        use crate::views::terminal::RowKind;
        let grid = pane.last_grid?;
        if pane.rows.is_empty() {
            return None;
        }
        // row under cursor, clamped to the visible slice
        let mut idx = (((my - grid.y) / grid.line_h).floor() as i64)
            .clamp(0, pane.rows.len() as i64 - 1) as usize;
        // clamp header rows to the nearest grid row
        if matches!(pane.rows[idx].kind, RowKind::Header(_)) {
            let down = pane.rows[idx..]
                .iter()
                .position(|r| matches!(r.kind, RowKind::Grid { .. }))
                .map(|o| idx + o);
            let up = pane.rows[..idx]
                .iter()
                .rposition(|r| matches!(r.kind, RowKind::Grid { .. }));
            idx = match (up, down) {
                (Some(u), Some(d)) => {
                    if my - pane.rows[u].y < pane.rows[d].y - my {
                        u
                    } else {
                        d
                    }
                }
                (Some(u), None) => u,
                (None, Some(d)) => d,
                (None, None) => return None,
            };
        }
        let RowKind::Grid { abs, content_x } = pane.rows[idx].kind else {
            return None;
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let line = ALine((abs as i64 - scrolled as i64) as i32);
        let colf = ((mx - content_x) / grid.cell_w).max(0.0);
        let col = (colf.floor() as usize).min(grid.cols.saturating_sub(1));
        let side = if colf.fract() >= 0.5 {
            ASide::Right
        } else {
            ASide::Left
        };
        Some((APoint::new(line, ACol(col)), side))
    }

    /// 1, 2, or 3 for single/double/triple click.
    fn click_count(&mut self) -> u8 {
        let now = Instant::now();
        let (mx, my) = self.mouse;
        let count = match self.last_click {
            Some((t, (px, py), c))
                if now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                    && (mx - px).abs() <= DRAG_THRESHOLD
                    && (my - py).abs() <= DRAG_THRESHOLD =>
            {
                if c >= 3 {
                    1
                } else {
                    c + 1
                }
            }
            _ => 1,
        };
        self.last_click = Some((now, (mx, my), count));
        count
    }

    /// Begin selection interaction on `pane` at the current mouse position.
    fn press_selection(&mut self, pane_id: PaneId, clicks: u8) {
        let shift = self.modifiers.shift_key();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        // one selection at a time across panes
        for p in &mut tab.panes {
            if p.session.id != pane_id {
                p.clear_selection();
            }
        }
        let Some(pane) = tab.pane_mut(pane_id) else {
            return;
        };
        // reserve the gesture for mouse-reporting apps unless shift is held
        use runway_pty::alacritty_terminal::term::TermMode;
        if !shift
            && pane
                .session
                .term
                .lock()
                .mode()
                .intersects(TermMode::MOUSE_MODE)
        {
            pane.clear_selection();
            return;
        }
        let (mx, my) = self.mouse;
        let Some((point, side)) = Self::grid_point_at(pane, mx, my) else {
            pane.clear_selection();
            return;
        };
        match clicks {
            2 | 3 => {
                let ty = if clicks == 2 {
                    SelectionType::Semantic
                } else {
                    SelectionType::Lines
                };
                pane.session.term.lock().selection = Some(Selection::new(ty, point, side));
                pane.has_selection = true;
                self.pending_sel = Some(PendingSelection {
                    pane: pane_id,
                    point,
                    side,
                    origin: (mx, my),
                    started: true,
                    block: false,
                });
            }
            _ => {
                // click clears; drag past threshold starts a Simple selection
                pane.clear_selection();
                self.pending_sel = Some(PendingSelection {
                    pane: pane_id,
                    point,
                    side,
                    origin: (mx, my),
                    started: false,
                    block: self.modifiers.alt_key(),
                });
            }
        }
    }

    /// Extend the pending/live selection to the current mouse position.
    fn drag_selection(&mut self) {
        let Some(pending) = &mut self.pending_sel else {
            return;
        };
        let (mx, my) = self.mouse;
        if !pending.started {
            let (ox, oy) = pending.origin;
            if (mx - ox).abs() < DRAG_THRESHOLD && (my - oy).abs() < DRAG_THRESHOLD {
                return;
            }
            pending.started = true;
        }
        let pane_id = pending.pane;
        let (anchor, anchor_side, block) = (pending.point, pending.side, pending.block);
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        let Some((point, side)) = Self::grid_point_at(pane, mx, my) else {
            return;
        };
        let mut term = pane.session.term.lock();
        if term.selection.is_none() {
            let ty = if block {
                SelectionType::Block
            } else {
                SelectionType::Simple
            };
            term.selection = Some(Selection::new(ty, anchor, anchor_side));
        }
        if let Some(sel) = &mut term.selection {
            sel.update(point, side);
        }
        drop(term);
        pane.has_selection = true;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Display-list index of a block's header (current collapse state).
    fn header_display_index(
        pane: &PaneState,
        block: runway_core::BlockId,
        hide_prompt: bool,
    ) -> Option<(usize, usize)> {
        use crate::views::terminal::{DisplayLine, build_display_list_filtered, filter_hidden};
        use runway_pty::alacritty_terminal::grid::Dimensions;
        let (blocks, clear_pad) = {
            let store = pane.session.blocks.lock().unwrap();
            (store.snapshot(), store.clear_pad())
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let (screen_lines, history) = {
            let term = pane.session.term.lock();
            let g = term.grid();
            (g.screen_lines(), g.history_size() as u64)
        };
        let bi = blocks.iter().position(|b| b.id == block)?;
        let hidden = filter_hidden(&pane.session, pane.filter.as_ref());
        let display = build_display_list_filtered(
            &blocks,
            scrolled,
            screen_lines,
            history,
            false,
            hide_prompt,
            clear_pad,
            &hidden,
        );
        let idx = display.iter().position(|d| *d == DisplayLine::Header(bi))?;
        Some((idx, display.len()))
    }

    /// Scroll so `block`'s header sits at the top of the viewport.
    fn jump_to_block(&mut self, pane_id: PaneId, block: runway_core::BlockId) {
        let editor_visible = self.editor_active;
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
        if let Some((idx, len)) = Self::header_display_index(pane, block, editor_visible) {
            let max_off = len.saturating_sub(rows);
            // view_offset counts from the bottom: header at top ⇒
            // offset = len - rows - idx (clamped)
            pane.view_offset =
                (len as i64 - rows as i64 - idx as i64).clamp(0, max_off as i64) as usize;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Display-list index of a block's last visible row (collapse-aware).
    fn block_end_display_index(
        pane: &PaneState,
        block: runway_core::BlockId,
        hide_prompt: bool,
    ) -> Option<(usize, usize)> {
        use crate::views::terminal::{DisplayLine, build_display_list_filtered, filter_hidden};
        use runway_pty::alacritty_terminal::grid::Dimensions;
        let (blocks, clear_pad) = {
            let store = pane.session.blocks.lock().unwrap();
            (store.snapshot(), store.clear_pad())
        };
        let scrolled = pane
            .session
            .scrolled
            .load(std::sync::atomic::Ordering::Acquire);
        let (screen_lines, history) = {
            let term = pane.session.term.lock();
            let g = term.grid();
            (g.screen_lines(), g.history_size() as u64)
        };
        let bi = blocks.iter().position(|b| b.id == block)?;
        let (start, end) = (blocks[bi].prompt_start, blocks[bi].end.unwrap_or(u64::MAX));
        let hidden = filter_hidden(&pane.session, pane.filter.as_ref());
        let display = build_display_list_filtered(
            &blocks,
            scrolled,
            screen_lines,
            history,
            false,
            hide_prompt,
            clear_pad,
            &hidden,
        );
        let idx = display.iter().rposition(|d| match d {
            DisplayLine::Header(i) => *i == bi,
            DisplayLine::Grid(abs) => *abs >= start && *abs < end,
            DisplayLine::Pad => false,
        })?;
        Some((idx, display.len()))
    }

    /// Scroll so `block`'s last row sits at the bottom of the viewport.
    fn jump_to_block_end(&mut self, pane_id: PaneId, block: runway_core::BlockId) {
        let editor_visible = self.editor_active;
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        if let Some((idx, len)) = Self::block_end_display_index(pane, block, editor_visible) {
            let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
            let max_off = len.saturating_sub(rows);
            // bottom visible row = len - 1 - view_offset  ⇒  put idx there
            pane.view_offset = (len as i64 - 1 - idx as i64).clamp(0, max_off as i64) as usize;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Jump to the next/prev bookmarked block relative to the viewport top.
    fn jump_bookmark(&mut self, dir: i64) {
        let editor_visible = self.editor_active;
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        let marks = pane.session.blocks.lock().unwrap().bookmarks();
        if marks.is_empty() {
            return;
        }
        // current top-of-viewport display index
        let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
        let len = pane.last_display_len;
        let top = len.saturating_sub(rows).saturating_sub(pane.view_offset);
        // pick first bookmark whose header index is beyond `top` in `dir`
        let mut best: Option<(usize, runway_core::BlockId)> = None;
        for id in marks {
            if let Some((idx, _)) = Self::header_display_index(pane, id, editor_visible) {
                let cmp = idx as i64 - top as i64;
                if (dir > 0 && cmp > 0) || (dir < 0 && cmp < 0) {
                    let d = cmp.unsigned_abs() as usize;
                    if best.is_none() || d < best.unwrap().0 {
                        best = Some((d, id));
                    }
                }
            }
        }
        if let Some((_, id)) = best {
            let pane_id = pane.session.id;
            self.jump_to_block(pane_id, id);
        }
    }

    /// Open the block context menu at `pos` (mouse position for right-click,
    /// header anchor for ⌃M).
    fn open_context_menu(&mut self, pane_id: PaneId, block: runway_core::BlockId, pos: (f32, f32)) {
        let Some(pane) = self.tabs.get(self.active).and_then(|t| t.pane(pane_id)) else {
            return;
        };
        let (collapsed, bookmarked, has_output, has_cwd, has_branch) = {
            let blocks = pane.session.blocks.lock().unwrap();
            let Some(b) = blocks.iter().find(|b| b.id == block) else {
                return;
            };
            (
                b.collapsed,
                b.bookmarked,
                b.output_range().is_some(),
                b.cwd.is_some(),
                b.branch.is_some(),
            )
        };
        let filtered = pane.filter.as_ref().is_some_and(|(b, _)| *b == block);
        let rerun_ok = self.editor_active;
        self.context_menu = ContextMenuState {
            open: true,
            pane: pane_id,
            block,
            pos,
            items: vec![
                MenuItem::new("Copy command", MenuAction::CopyCommand),
                MenuItem::new("Copy output", MenuAction::CopyOutput).enabled(has_output),
                MenuItem::new("Copy both", MenuAction::CopyBoth).enabled(has_output),
                MenuItem::new("Copy working directory", MenuAction::CopyCwd).enabled(has_cwd),
                MenuItem::new("Copy git branch", MenuAction::CopyBranch).enabled(has_branch),
                MenuItem::new("Re-run command", MenuAction::Rerun)
                    .enabled(rerun_ok)
                    .sep(),
                MenuItem::new(
                    if filtered {
                        "Clear output filter"
                    } else {
                        "Filter output…"
                    },
                    MenuAction::FilterBlock,
                )
                .enabled(has_output),
                MenuItem::new(
                    if bookmarked {
                        "Remove bookmark"
                    } else {
                        "Bookmark"
                    },
                    MenuAction::ToggleBookmark,
                ),
                MenuItem::new(
                    if collapsed { "Expand" } else { "Collapse" },
                    MenuAction::ToggleCollapse,
                )
                .enabled(has_output),
                MenuItem::new("Scroll to top of block", MenuAction::ScrollTopOfBlock).sep(),
                MenuItem::new("Scroll to bottom of block", MenuAction::ScrollBottomOfBlock),
            ],
            selected: None,
        };
    }

    /// Right-click outside any block: clipboard, splits, clear, close.
    fn open_terminal_menu(&mut self, pane_id: PaneId, pos: (f32, f32)) {
        let has_sel = self
            .tabs
            .get(self.active)
            .and_then(|t| t.pane(pane_id))
            .is_some_and(|p| p.has_selection);
        self.context_menu = ContextMenuState {
            open: true,
            pane: pane_id,
            block: 0,
            pos,
            items: vec![
                MenuItem::new("Copy", MenuAction::Copy).enabled(has_sel),
                MenuItem::new("Paste", MenuAction::Paste),
                MenuItem::new("Select all", MenuAction::SelectAll),
                MenuItem::new("Split pane right", MenuAction::SplitRight).sep(),
                MenuItem::new("Split pane down", MenuAction::SplitDown),
                MenuItem::new("Clear blocks", MenuAction::ClearBlocks).sep(),
                MenuItem::new("Close pane", MenuAction::ClosePane),
            ],
            selected: None,
        };
    }

    /// Right-click a tab: rename, duplicate, color, sync, close variants.
    fn open_tab_menu(&mut self, tab: usize, pos: (f32, f32)) {
        let Some(t) = self.tabs.get(tab) else { return };
        let n = self.tabs.len();
        let split = t.is_split();
        let sync = t.sync_inputs;
        let mut items = vec![
            MenuItem::new("Rename tab", MenuAction::RenameTab(tab)),
            MenuItem::new("Duplicate tab", MenuAction::DuplicateTab(tab)),
            MenuItem::new(
                if sync {
                    "Stop synchronizing inputs"
                } else {
                    "Synchronize inputs in panes"
                },
                MenuAction::ToggleSync(tab),
            )
            .enabled(split || sync),
        ];
        for (i, name) in ["Red", "Green", "Yellow", "Blue", "Magenta", "Cyan"]
            .into_iter()
            .enumerate()
        {
            let item = MenuItem::new(name, MenuAction::SetTabColor(tab, Some(i)));
            items.push(if i == 0 { item.sep() } else { item });
        }
        items.push(
            MenuItem::new("No color", MenuAction::SetTabColor(tab, None))
                .enabled(t.color.is_some()),
        );
        items.push(MenuItem::new("Close tab", MenuAction::CloseTab(tab)).sep());
        items.push(
            MenuItem::new("Close other tabs", MenuAction::CloseOtherTabs(tab)).enabled(n > 1),
        );
        items.push(
            MenuItem::new("Close tabs to the right", MenuAction::CloseTabsRight(tab))
                .enabled(tab + 1 < n),
        );
        self.context_menu = ContextMenuState {
            open: true,
            pane: 0,
            block: 0,
            pos,
            items,
            selected: None,
        };
    }

    /// Keyboard in an open context menu: ↑/↓ move, Enter runs, Esc closes.
    /// Returns false when the key should close the menu.
    fn handle_menu_key(&mut self, event: &input::KeyPress, event_loop: &ActiveEventLoop) -> bool {
        use winit::keyboard::{Key, NamedKey};
        match event.logical_key {
            Key::Named(NamedKey::ArrowDown) | Key::Named(NamedKey::Tab) => {
                self.context_menu.step(1);
                true
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.context_menu.step(-1);
                true
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(i) = self.context_menu.selected
                    && let Some(item) = self.context_menu.items.get(i)
                    && item.enabled
                {
                    let action = item.action;
                    self.run_menu_action(action, event_loop);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn run_menu_action(&mut self, action: MenuAction, event_loop: &ActiveEventLoop) {
        let pane_id = self.context_menu.pane;
        let block = self.context_menu.block;
        self.context_menu.close();
        let editor_active = self.editor_active;

        // actions that are not about one block
        match action {
            MenuAction::Copy => return self.handle_action(AppAction::Copy, event_loop),
            MenuAction::Paste => return self.handle_action(AppAction::Paste, event_loop),
            MenuAction::SelectAll => return self.handle_action(AppAction::SelectAll, event_loop),
            MenuAction::SplitRight => return self.handle_action(AppAction::SplitRight, event_loop),
            MenuAction::SplitDown => return self.handle_action(AppAction::SplitDown, event_loop),
            MenuAction::ClearBlocks => {
                return self.handle_action(AppAction::ClearBlocks, event_loop);
            }
            MenuAction::ClosePane => {
                return self.request_close(CloseScope::Pane(pane_id), event_loop);
            }
            MenuAction::RenameTab(i) => {
                self.start_rename(i);
                return;
            }
            MenuAction::DuplicateTab(i) => {
                let cwd = self
                    .tabs
                    .get(i)
                    .and_then(|t| t.active())
                    .and_then(|p| p.session.cwd.lock().unwrap().clone())
                    .map(std::path::PathBuf::from);
                self.spawn_tab_at(cwd.filter(|p| p.is_dir()));
                // keep the copy next to its source
                let new = self.tabs.len() - 1;
                if i + 1 < new {
                    let tab = self.tabs.remove(new);
                    self.tabs.insert(i + 1, tab);
                    self.active = i + 1;
                }
                self.save_session();
                return;
            }
            MenuAction::ToggleSync(i) => {
                if let Some(t) = self.tabs.get_mut(i) {
                    t.sync_inputs = !t.sync_inputs;
                }
                return;
            }
            MenuAction::SetTabColor(i, c) => {
                if let Some(t) = self.tabs.get_mut(i) {
                    t.color = c;
                }
                self.save_session();
                return;
            }
            MenuAction::CloseTab(i) => return self.request_close(CloseScope::Tab(i), event_loop),
            MenuAction::CloseOtherTabs(i) => {
                return self.request_close(CloseScope::OtherTabs(i), event_loop);
            }
            MenuAction::CloseTabsRight(i) => {
                // close from the end so indices stay valid
                for t in (i + 1..self.tabs.len()).rev() {
                    self.request_close(CloseScope::Tab(t), event_loop);
                    if self.confirm.is_some() {
                        break;
                    }
                }
                return;
            }
            MenuAction::FilterBlock => {
                self.toggle_block_filter(pane_id, block);
                return;
            }
            _ => {}
        }

        // gather everything under a short borrow, then act on self
        let mut to_copy: Option<String> = None;
        let mut jump_top = false;
        let mut jump_bottom = false;
        let mut rerun: Option<String> = None;
        {
            let Some(pane) = self
                .tabs
                .get_mut(self.active)
                .and_then(|t| t.pane_mut(pane_id))
            else {
                return;
            };
            let (command, cwd, branch) = pane
                .session
                .blocks
                .lock()
                .unwrap()
                .iter()
                .find(|b| b.id == block)
                .map(|b| (b.command.clone(), b.cwd.clone(), b.branch.clone()))
                .unwrap_or_default();
            match action {
                MenuAction::CopyCommand => to_copy = Some(command),
                MenuAction::CopyCwd => to_copy = cwd,
                MenuAction::CopyBranch => to_copy = branch,
                MenuAction::ScrollTopOfBlock => jump_top = true,
                MenuAction::ScrollBottomOfBlock => jump_bottom = true,
                MenuAction::CopyOutput => {
                    to_copy = terminal::block_output_text(&pane.session, block);
                }
                MenuAction::CopyBoth => {
                    let output =
                        terminal::block_output_text(&pane.session, block).unwrap_or_default();
                    to_copy = Some(format!("{command}\n{output}"));
                }
                MenuAction::Rerun => {
                    if !command.is_empty() && editor_active {
                        rerun = Some(command);
                    }
                }
                MenuAction::ToggleBookmark => {
                    pane.session.blocks.lock().unwrap().toggle_bookmark(block);
                }
                MenuAction::ToggleCollapse => {
                    pane.session.blocks.lock().unwrap().toggle_collapsed(block);
                }
                _ => {}
            }
        }
        if let Some(cmd) = rerun {
            self.submit_to(pane_id, &cmd);
        }
        if let Some(text) = to_copy {
            self.copy_to_clipboard(text);
        }
        if jump_top {
            self.jump_to_block(pane_id, block);
        }
        if jump_bottom {
            self.jump_to_block_end(pane_id, block);
        }
    }

    /// Block filter (Warp): open the filter bar for `block`, or clear the
    /// filter when that block is already filtered.
    fn toggle_block_filter(&mut self, pane_id: PaneId, block: runway_core::BlockId) {
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        if pane.filter.as_ref().is_some_and(|(b, _)| *b == block) {
            pane.filter = None;
            self.filter_editing = false;
            self.search.close();
            return;
        }
        pane.filter = Some((block, String::new()));
        pane.selected_block = Some(block);
        self.search.close();
        self.search.open = true;
        self.filter_editing = true;
        self.blink_epoch = Instant::now();
        self.jump_to_block(pane_id, block);
    }

    /// Command text of `block` (None when missing or empty).
    fn block_command(&self, pane_id: PaneId, block: runway_core::BlockId) -> Option<String> {
        let pane = self.tabs.get(self.active)?.pane(pane_id)?;
        let cmd = pane
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.id == block)
            .map(|b| b.command.clone())?;
        (!cmd.is_empty()).then_some(cmd)
    }

    /// Target for block-scoped shortcuts: the keyboard-selected block when
    /// still present, else the most recent finished block of the active pane.
    fn selected_or_last_block(&self) -> Option<(PaneId, runway_core::BlockId)> {
        let pane = self.tabs.get(self.active)?.active()?;
        let blocks = pane.session.blocks.lock().unwrap();
        if let Some(sel) = pane.selected_block
            && blocks.iter().any(|b| b.id == sel)
        {
            return Some((pane.session.id, sel));
        }
        blocks
            .iter()
            .rev()
            .find(|b| b.suppressed_range().is_some())
            .map(|b| (pane.session.id, b.id))
    }

    /// ⌘↑ / ⌘↓: move the keyboard block selection; past the newest block the
    /// selection clears and the view snaps back to the input.
    fn select_block(&mut self, dir: i64) {
        let editor_visible = self.editor_active;
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        let eligible: Vec<runway_core::BlockId> = pane
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| b.suppressed_range().is_some())
            .map(|b| b.id)
            .collect();
        if eligible.is_empty() {
            return;
        }
        let cur = pane.selected_block.filter(|id| eligible.contains(id));
        let next = match (cur, dir) {
            (None, d) if d < 0 => Some(*eligible.last().unwrap()),
            (None, _) => None,
            (Some(id), d) => {
                let i = eligible.iter().position(|b| *b == id).unwrap() as i64 + d;
                if i < 0 {
                    Some(eligible[0]) // clamp at the oldest
                } else if (i as usize) < eligible.len() {
                    Some(eligible[i as usize])
                } else {
                    None // past the newest → back to the input
                }
            }
        };
        pane.selected_block = next;
        match next {
            Some(id) => {
                let pane_id = pane.session.id;
                let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
                let len = pane.last_display_len;
                let top = len.saturating_sub(rows).saturating_sub(pane.view_offset);
                let offscreen = Self::header_display_index(pane, id, editor_visible)
                    .is_some_and(|(idx, _)| idx < top || idx >= top + rows);
                if offscreen {
                    self.jump_to_block(pane_id, id);
                }
            }
            None => pane.view_offset = 0,
        }
    }

    /// cmd+click on a detected URL opens it. Returns true when handled.
    fn try_open_url(&mut self, pane_id: PaneId) -> bool {
        if !self.modifiers.super_key() {
            return false;
        }
        let (mx, my) = self.mouse;
        let url = self
            .tabs
            .get(self.active)
            .and_then(|t| t.pane(pane_id))
            .and_then(|p| {
                p.url_hits
                    .iter()
                    .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
                    .map(|(_, u)| u.clone())
            });
        let target = match url {
            Some(url) => {
                let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
                // only schemes that open documents, never script handlers
                if !matches!(
                    scheme.as_deref(),
                    Some("http" | "https" | "file" | "mailto" | "ftp")
                ) {
                    return false;
                }
                url
            }
            None => match self.path_under_mouse(pane_id) {
                Some(path) => path,
                None => return false,
            },
        };
        #[cfg(target_os = "macos")]
        let opener = "open";
        #[cfg(not(target_os = "macos"))]
        let opener = "xdg-open";
        log::info!("open {target}");
        let _ = std::process::Command::new(opener)
            .arg(&target)
            .stdin(std::process::Stdio::null())
            .spawn();
        true
    }

    /// An existing file or directory named by the text under the pointer
    /// (absolute, `~/…`, or relative to the pane's working directory;
    /// `file:line:col` suffixes are dropped).
    fn path_under_mouse(&self, pane_id: PaneId) -> Option<String> {
        use crate::views::terminal::RowKind;
        let pane = self.tabs.get(self.active)?.pane(pane_id)?;
        let g = pane.last_grid?;
        let (mx, my) = self.mouse;
        let row = pane
            .rows
            .iter()
            .find(|r| my >= r.y && my < r.y + g.line_h)?;
        let RowKind::Grid { abs, content_x } = row.kind else {
            return None;
        };
        let col = ((mx - content_x) / g.cell_w).floor();
        if col < 0.0 {
            return None;
        }
        let text = terminal::session_line_text(&pane.session, abs)?;
        let (path, _line) = terminal::path_token_at(&text, col as usize)?;
        let home = std::env::var("HOME").unwrap_or_default();
        let resolved = if let Some(rest) = path.strip_prefix("~/") {
            std::path::Path::new(&home).join(rest)
        } else if path.starts_with('/') {
            std::path::PathBuf::from(&path)
        } else {
            // block output ran in the block's cwd; the pane's is the best guess
            let cwd = pane.session.cwd.lock().unwrap().clone()?;
            std::path::Path::new(&cwd).join(&path)
        };
        resolved
            .exists()
            .then(|| resolved.to_string_lossy().into_owned())
    }

    /// Pointer shape: a hand over ⌘-clickable links, the I-beam over text.
    fn update_cursor_icon(&mut self, window: &Window) {
        use winit::window::CursorIcon;
        let (mx, my) = self.mouse;
        let over_link = self.modifiers.super_key()
            && self.tabs.get(self.active).is_some_and(|t| {
                t.panes.iter().any(|p| {
                    p.url_hits.iter().any(|(r, _)| {
                        mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
                    })
                })
            });
        let over_path = !over_link
            && self.modifiers.super_key()
            && self
                .pane_under_mouse()
                .is_some_and(|id| self.path_under_mouse(id).is_some());
        let icon = if over_link || over_path {
            CursorIcon::Pointer
        } else if my > TAB_BAR_H && !self.settings.open && !self.palette.open {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };
        if self.cursor_icon != icon {
            self.cursor_icon = icon;
            window.set_cursor(icon);
        }
    }

    /// Selected text of the active pane, if any.
    fn selection_text(&self) -> Option<String> {
        let pane = self.tabs.get(self.active)?.active()?;
        log::debug!("selection_text: has_selection={}", pane.has_selection);
        if !pane.has_selection {
            return None;
        }
        let text = pane.session.term.lock().selection_to_string();
        log::debug!("selection_text: got={:?}", text);
        text.filter(|t| !t.is_empty())
    }

    fn copy_to_clipboard(&mut self, text: String) {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        if let Some(cb) = &mut self.clipboard
            && let Err(e) = cb.set_text(text)
        {
            log::warn!("clipboard set failed: {e}");
        }
    }

    fn paste_from_clipboard(&mut self) {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        let Some(text) = self.clipboard.as_mut().and_then(|cb| cb.get_text().ok()) else {
            return;
        };
        self.paste_text(&text);
    }

    /// Paste `text` into the active pane: the owned editor when it is live,
    /// else the PTY (bracketed when the app asked for it; mirrored to
    /// synchronized panes).
    fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let editor_active = self.editor_active;
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        if editor_active {
            pane.editor.insert(text);
            pane.selected_block = None;
            self.update_completions();
            return;
        }
        use runway_pty::alacritty_terminal::term::TermMode;
        let bracketed = pane
            .session
            .term
            .lock()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        let id = pane.session.id;
        let mut bytes = Vec::with_capacity(text.len() + 12);
        if bracketed {
            // a pasted end marker must not terminate the paste early
            bytes.extend_from_slice(b"\x1b[200~");
            bytes.extend_from_slice(text.replace("\x1b[201~", "").as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
        } else {
            // unbracketed: newlines submit, like typing them would
            bytes.extend_from_slice(text.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
        }
        self.write_synced(id, &bytes, false);
    }

    /// Files dropped on the window: their shell-quoted paths are pasted.
    fn drop_file(&mut self, path: &std::path::Path) {
        let mut text = input::shell_quote(&path.to_string_lossy());
        text.push(' ');
        self.paste_text(&text);
    }

    /// Re-run the search over the active pane and jump near the latest match.
    fn refresh_search(&mut self, jump: bool) {
        let Some(pane) = self.tabs.get(self.active).and_then(|t| t.active()) else {
            self.search.matches.clear();
            return;
        };
        self.search.matches = match self.search.matcher() {
            Some(m) => terminal::collect_matches(&pane.session, &m, 500),
            None => Vec::new(),
        };
        self.search.current = 0;
        if !self.search.matches.is_empty() {
            self.search.current = self.search.matches.len() - 1;
            if jump {
                self.jump_to_match();
            }
        }
    }

    fn jump_to_match(&mut self) {
        let Some((abs, _)) = self.search.current_anchor() else {
            return;
        };
        if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
            use runway_pty::alacritty_terminal::grid::Dimensions;
            use std::sync::atomic::Ordering;
            let scrolled = pane.session.scrolled.load(Ordering::Acquire);
            let screen_lines = pane.session.term.lock().grid().screen_lines() as u64;
            let newest = scrolled + screen_lines;
            pane.view_offset = (newest.saturating_sub(abs) as usize).saturating_sub(12);
        }
    }

    /// Search-mode keyboard handling.
    fn handle_search_key(&mut self, event: &input::KeyPress, mods: ModifiersState) {
        use winit::keyboard::{Key, NamedKey};
        let shift = mods.shift_key();
        // ⌥C match case, ⌥R regex (physical key: Option composes on macOS)
        if mods.alt_key() && !mods.super_key() {
            let key = match (&event.unmodified, &event.logical_key) {
                (Some(Key::Character(s)), _) | (None, Key::Character(s)) => s.to_lowercase(),
                _ => String::new(),
            };
            match key.as_str() {
                "c" => self.search.case_sensitive = !self.search.case_sensitive,
                "r" => self.search.regex = !self.search.regex,
                _ => return,
            }
            self.refresh_search(true);
            return;
        }
        // cmd chords: ⌘G / ⇧⌘G cycle matches; everything else is swallowed
        // (never appended to the query as text)
        if mods.super_key() {
            if let Key::Character(s) = &event.logical_key
                && s.to_ascii_lowercase() == "g"
            {
                if shift {
                    self.search.prev();
                } else {
                    self.search.next();
                }
                self.jump_to_match();
                self.blink_epoch = Instant::now();
            }
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.search.close();
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    pane.view_offset = 0;
                }
            }
            Key::Named(NamedKey::Enter) => {
                if shift {
                    self.search.prev();
                } else {
                    self.search.next();
                }
                self.jump_to_match();
            }
            Key::Named(NamedKey::Backspace) => {
                self.search.query.pop();
                self.refresh_search(true);
            }
            Key::Named(NamedKey::Space) => {
                self.search.query.push(' ');
                self.refresh_search(true);
            }
            _ => {
                if let Some(text) = &event.text
                    && !text.is_empty()
                    && !text.chars().any(|c| c.is_control())
                {
                    self.search.query.push_str(text);
                    self.refresh_search(true);
                }
            }
        }
        self.blink_epoch = Instant::now();
    }

    /// Block-filter input: type to narrow, Enter keeps the filter, Esc (or
    /// the filter shortcut) clears it.
    fn handle_filter_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let action = input::app_action(event, self.modifiers, &self.keymap);
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            self.filter_editing = false;
            self.search.close();
            return;
        };
        let Some((block, query)) = pane.filter.as_mut() else {
            self.filter_editing = false;
            self.search.close();
            return;
        };
        let block = *block;
        match &event.logical_key {
            _ if action == Some(AppAction::FilterBlock) => {
                pane.filter = None;
                self.filter_editing = false;
                self.search.close();
            }
            Key::Named(NamedKey::Escape) => {
                pane.filter = None;
                self.filter_editing = false;
                self.search.close();
            }
            Key::Named(NamedKey::Enter) => {
                if query.is_empty() {
                    pane.filter = None;
                }
                self.filter_editing = false;
                self.search.close();
            }
            Key::Named(NamedKey::Backspace) => {
                query.pop();
            }
            Key::Named(NamedKey::Space) => query.push(' '),
            _ => {
                if !self.modifiers.super_key()
                    && !self.modifiers.control_key()
                    && let Some(text) = &event.text
                    && !text.chars().any(|c| c.is_control())
                {
                    query.push_str(text);
                }
            }
        }
        let pane_id = pane.session.id;
        if self.filter_editing {
            // keep the filtered block's header in view while narrowing
            self.jump_to_block(pane_id, block);
        }
        self.blink_epoch = Instant::now();
    }

    /// Same list (and order) `SwitchTheme(i)` indexes: builtins, then user
    /// themes — the palette used to list builtins only.
    fn theme_names() -> Vec<String> {
        runway_core::theme::available_themes()
    }

    fn refresh_palette(&mut self) {
        use crate::views::palette::PaletteMode;
        let pane_history = self
            .tabs
            .get(self.active)
            .and_then(|t| t.active())
            .map(|p| p.editor.history.clone())
            .unwrap_or_default();
        let (items, recent) = match self.palette.mode {
            PaletteMode::Commands => {
                let launches: Vec<String> =
                    self.launch_configs.iter().map(|l| l.name.clone()).collect();
                (
                    palette::base_items(&Self::theme_names(), &launches, &self.keymap),
                    pane_history,
                )
            }
            PaletteMode::History => {
                let mut all = self.shell_history.clone();
                all.extend(pane_history);
                (palette::history_items(&all), Vec::new())
            }
            PaletteMode::Workflows => (palette::workflow_items(&self.workflows), Vec::new()),
        };
        self.palette_results = palette::rank(&items, &recent, &self.palette.query);
        if self.palette.selected >= self.palette_results.len() {
            self.palette.selected = self.palette_results.len().saturating_sub(1);
        }
    }

    fn open_palette(&mut self, mode: crate::views::palette::PaletteMode) {
        if mode == crate::views::palette::PaletteMode::Commands {
            // pick up files added while running
            self.launch_configs = runway_core::launch::load_all();
        }
        self.palette.open = true;
        self.palette.mode = mode;
        self.palette.query.clear();
        self.palette.selected = 0;
        self.refresh_palette();
        self.blink_epoch = Instant::now();
    }

    fn run_palette_action(&mut self, action: PaletteAction, event_loop: &ActiveEventLoop) {
        self.palette.open = false;
        match action {
            PaletteAction::App(a) => self.handle_action(a, event_loop),
            PaletteAction::OpenSettings => {
                self.settings.open = true;
            }
            PaletteAction::SwitchTheme(i) => {
                let names = runway_core::theme::available_themes();
                if let Some(name) = names.into_iter().nth(i) {
                    self.config.theme = name;
                    self.save_config();
                    self.apply_config();
                }
            }
            PaletteAction::Search => {
                self.search.open = true;
                self.refresh_search(true);
            }
            PaletteAction::OpenHistory => {
                self.open_palette(crate::views::palette::PaletteMode::History);
            }
            PaletteAction::ToggleVim => {
                self.config.vim_mode = !self.config.vim_mode;
                self.save_config();
                self.apply_config();
            }
            PaletteAction::Launch(i) => {
                if let Some(cfg) = self.launch_configs.get(i).cloned() {
                    self.apply_launch_config(&cfg);
                }
            }
            PaletteAction::InsertCommand(cmd) => {
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    pane.editor.clear();
                    pane.editor.insert(&cmd);
                }
                self.update_completions();
            }
            PaletteAction::InsertWorkflow(i) => {
                if let Some((cmd, spans)) = self.workflows.get(i).map(|w| w.instantiate()) {
                    if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
                    {
                        pane.editor.set_with_placeholders(cmd, spans);
                    }
                    self.completion.close();
                }
            }
            PaletteAction::RunHistory(cmd) => {
                // same path as Enter: prompt-time context, synced panes
                if let Some(id) = self.tabs.get(self.active).map(|t| t.active_pane) {
                    for target in self.input_targets(id) {
                        self.submit_to(target, &cmd);
                    }
                }
            }
        }
    }

    /// Palette-mode keyboard handling. Returns true when consumed.
    fn handle_palette_key(
        &mut self,
        event: &input::KeyPress,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        use winit::keyboard::{Key, NamedKey};
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.palette.open = false;
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(m) = self.palette_results.get(self.palette.selected) {
                    let action = m.item.action.clone();
                    self.run_palette_action(action, event_loop);
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.palette.selected = self.palette.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.palette.selected + 1 < self.palette_results.len() {
                    self.palette.selected += 1;
                }
            }
            Key::Named(NamedKey::Backspace) => {
                self.palette.query.pop();
                self.palette.selected = 0;
                self.refresh_palette();
            }
            Key::Named(NamedKey::Space) => {
                self.palette.query.push(' ');
                self.palette.selected = 0;
                self.refresh_palette();
            }
            _ => {
                if let Some(text) = &event.text
                    && !text.is_empty()
                    && !text.chars().any(|c| c.is_control())
                {
                    self.palette.query.push_str(text);
                    self.palette.selected = 0;
                    self.refresh_palette();
                }
            }
        }
        self.blink_epoch = Instant::now();
        true
    }

    /// Recompute completions + ghost for the active editor.
    fn update_completions(&mut self) {
        let Some(pane) = self.tabs.get(self.active).and_then(|t| t.active()) else {
            self.completion.close();
            return;
        };
        if pane.editor.in_placeholder_mode() {
            self.completion.close();
            return;
        }
        let buffer = pane.editor.buffer.clone();
        let cursor = pane.editor.cursor;
        if buffer.trim().is_empty() {
            self.completion.close();
            return;
        }
        let cwd = pane.session.cwd.lock().unwrap().clone();
        let items = runway_core::completion::complete(
            &buffer,
            cursor,
            &pane.editor.history,
            &self.shell_history,
            cwd.as_deref(),
        );

        // ghost: best history match remainder, else the single completion
        self.completion.ghost = items
            .iter()
            .find(|c| c.source == "history")
            .and_then(|c| c.insert.strip_prefix(&buffer).map(str::to_string))
            .unwrap_or_default();

        // dropdown shows non-history items (history drives ghost text)
        let dropdown: Vec<_> = items
            .into_iter()
            .filter(|c| c.source != "history")
            .collect();
        if self.completion.ghost.is_empty()
            && let Some(first) = dropdown.first()
        {
            let (_, token) = runway_core::completion::current_token(&buffer, cursor);
            if cursor == buffer.len() {
                self.completion.ghost = first
                    .insert
                    .strip_prefix(token)
                    .unwrap_or_default()
                    .to_string();
            }
        }
        if !self.config.autosuggestions {
            self.completion.ghost.clear();
        }
        self.completion.items = if self.config.completions_menu {
            dropdown
        } else {
            Vec::new()
        };
        self.completion.selected = 0;
        self.completion.open = !self.completion.items.is_empty();
    }

    /// ⌃→: accept one word of the ghost suggestion.
    fn accept_ghost_word(&mut self) {
        if self.completion.ghost.is_empty() {
            return;
        }
        let ghost = self.completion.ghost.clone();
        // word = leading whitespace + next non-space run
        let trimmed = ghost.trim_start();
        let lead = ghost.len() - trimmed.len();
        let word_end = trimmed
            .find(char::is_whitespace)
            .map(|i| lead + i)
            .unwrap_or(ghost.len());
        let word = &ghost[..word_end];
        if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
            pane.editor.end();
            pane.editor.insert(word);
        }
        self.update_completions();
    }

    /// Accept the selected completion (Tab).
    fn accept_completion(&mut self) {
        let (replacement, ghost) = if self.completion.open {
            (
                self.completion
                    .items
                    .get(self.completion.selected)
                    .map(|c| c.insert.clone()),
                None,
            )
        } else if !self.completion.ghost.is_empty() {
            (None, Some(self.completion.ghost.clone()))
        } else {
            return;
        };
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        if let Some(ghost) = ghost {
            pane.editor.end();
            pane.editor.insert(&ghost);
        } else if let Some(insert) = replacement {
            let buffer = pane.editor.buffer.clone();
            let cursor = pane.editor.cursor;
            let (start, _) = runway_core::completion::current_token(&buffer, cursor);
            // through the single mutation point: keeps placeholder spans
            // valid and makes the completion undoable
            pane.editor.replace_range(start..cursor, &insert);
        }
        self.completion.close();
        self.update_completions();
    }

    fn logical_size(&self) -> (f32, f32) {
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        let gpu = self.gpu.as_ref().unwrap();
        (
            gpu.config.width as f32 / scale,
            gpu.config.height as f32 / scale,
        )
    }

    /// Editor is shown when shell integration is live (last block is a
    /// prompt) and no full-screen app is running.
    fn editor_visible(pane: &PaneState) -> bool {
        use runway_pty::alacritty_terminal::term::TermMode;
        let alt = pane
            .session
            .term
            .lock()
            .mode()
            .contains(TermMode::ALT_SCREEN);
        if alt {
            return false;
        }
        matches!(
            pane.session
                .blocks
                .lock()
                .unwrap()
                .last()
                .map(|b| b.state.clone()),
            Some(BlockState::Prompt)
        )
    }

    fn spawn_session(
        &mut self,
        cols: usize,
        rows: usize,
        cwd: Option<std::path::PathBuf>,
    ) -> Session {
        let id = self.next_pane;
        self.next_pane += 1;

        let proxy = self.proxy.clone();
        let observer: runway_pty::Observer = Arc::new(move |pane, event| {
            let _ = match event {
                SessionEvent::Wakeup => proxy.send_event(UserEvent::PtyWakeup(pane)),
                SessionEvent::Title(t) => proxy.send_event(UserEvent::PtyTitle(pane, t)),
                SessionEvent::ResetTitle => {
                    proxy.send_event(UserEvent::PtyTitle(pane, String::new()))
                }
                SessionEvent::Clipboard(text) => proxy.send_event(UserEvent::PtyClipboard(text)),
                SessionEvent::ColorRequest(index, reply) => {
                    proxy.send_event(UserEvent::PtyColorRequest(pane, index, reply))
                }
                SessionEvent::Exit => proxy.send_event(UserEvent::PtyExit(pane)),
                SessionEvent::Bell => proxy.send_event(UserEvent::PtyBell(pane)),
            };
        });
        Session::spawn(SpawnOptions {
            id,
            size: GridSize { cols, rows },
            cell_size: (self.cell_w.round() as u16, self.line_h.round() as u16),
            shell: self.config.shell.clone(),
            working_directory: cwd,
            env: vec![],
            scrollback: self.config.scrollback,
            observer,
        })
        .expect("spawn shell")
    }

    fn spawn_tab(&mut self) {
        let dir = self.start_dir();
        self.spawn_tab_at(dir);
    }

    /// Probe the user's shell for aliases/functions off the UI thread.
    fn load_shell_env(&self) {
        let proxy = self.proxy.clone();
        let shell = runway_core::shell_integration::resolve_shell(self.config.shell.as_deref());
        std::thread::Builder::new()
            .name("shell-env".into())
            .spawn(move || {
                let env = runway_core::shell_env::ShellEnv::load(&shell);
                let _ = proxy.send_event(UserEvent::ShellEnvLoaded(Arc::new(env)));
            })
            .ok();
    }

    fn spawn_tab_at(&mut self, cwd: Option<std::path::PathBuf>) {
        let (lw, lh) = self.logical_size();
        let cols = ((lw - self.config.padding() * 2.0 - terminal::BLOCK_INDENT) / self.cell_w)
            .max(2.0) as usize;
        let rows = ((lh - TAB_BAR_H - TOP_PAD - EDITOR_H) / self.line_h).max(2.0) as usize;
        let session = self.spawn_session(cols, rows, cwd);
        self.tabs.push(TabState::new(PaneState::new(session)));
        self.active = self.tabs.len() - 1;
        self.save_session();
    }

    /// Persist tabs: split layouts, cwds, custom titles, colors (atomic;
    /// called on every structural change).
    fn save_session(&self) {
        use runway_core::session_state::{SavedAxis, SavedLayout, SavedSession, SavedTab, save};
        if !self.persist_session {
            return;
        }
        fn pane_cwd(p: &PaneState) -> Option<String> {
            p.session
                .cwd
                .lock()
                .unwrap()
                .clone()
                .or_else(|| p.last_git_cwd.clone())
        }
        fn layout(tab: &TabState, node: &crate::pane::PaneNode) -> SavedLayout {
            match node {
                crate::pane::PaneNode::Leaf(id) => SavedLayout::Pane {
                    cwd: tab.pane(*id).and_then(pane_cwd),
                    focused: *id == tab.active_pane,
                },
                crate::pane::PaneNode::Split {
                    axis,
                    ratio,
                    first,
                    second,
                } => SavedLayout::split(
                    match axis {
                        Axis::Horizontal => SavedAxis::Horizontal,
                        Axis::Vertical => SavedAxis::Vertical,
                    },
                    *ratio,
                    layout(tab, first),
                    layout(tab, second),
                ),
            }
        }
        let tabs = self
            .tabs
            .iter()
            .map(|t| SavedTab {
                cwd: t.active().and_then(pane_cwd),
                title: t.custom_title.clone(),
                color: t
                    .color
                    .and_then(|c| crate::state::TAB_COLORS.get(c))
                    .map(|s| s.to_string()),
                layout: t.is_split().then(|| layout(t, &t.root)),
            })
            .collect();
        save(&SavedSession {
            tabs,
            active: self.active,
            window: self.window_frame(),
            ..Default::default()
        });
    }

    /// Current window frame in logical px (for session restore).
    fn window_frame(&self) -> Option<runway_core::session_state::SavedWindow> {
        let w = self.window.as_ref()?;
        let scale = w.scale_factor();
        let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
        let size = w.inner_size().to_logical::<f64>(scale);
        Some(runway_core::session_state::SavedWindow {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized: w.is_maximized(),
            fullscreen: w.fullscreen().is_some(),
        })
    }

    /// Reopen a saved tab: its split tree, pane cwds, title and color.
    fn restore_tab(&mut self, saved: &runway_core::session_state::SavedTab) {
        use runway_core::session_state::SavedLayout;
        fn first_cwd(l: &SavedLayout) -> Option<std::path::PathBuf> {
            match l {
                SavedLayout::Pane { cwd, .. } => cwd.clone().map(std::path::PathBuf::from),
                SavedLayout::Split { first, .. } => first_cwd(first),
            }
            .filter(|p| p.is_dir())
        }
        let top_cwd = saved.layout.as_ref().and_then(first_cwd).or_else(|| {
            saved
                .cwd
                .clone()
                .map(std::path::PathBuf::from)
                .filter(|p| p.is_dir())
        });
        self.spawn_tab_at(top_cwd);
        let root = self.tabs[self.active].active_pane;
        if let Some(layout) = &saved.layout {
            self.build_saved(layout, root);
            let tab = &mut self.tabs[self.active];
            apply_ratios(&mut tab.root, layout);
            if !tab.panes.iter().any(|p| p.session.id == tab.active_pane) {
                tab.active_pane = root;
            }
        }
        let tab = &mut self.tabs[self.active];
        tab.custom_title = saved.title.clone().filter(|t| !t.is_empty());
        tab.color = saved
            .color
            .as_deref()
            .and_then(|c| crate::state::TAB_COLORS.iter().position(|n| *n == c));

        fn apply_ratios(node: &mut crate::pane::PaneNode, saved: &SavedLayout) {
            if let (
                crate::pane::PaneNode::Split {
                    ratio,
                    first,
                    second,
                    ..
                },
                SavedLayout::Split {
                    ratio: r,
                    first: f,
                    second: s,
                    ..
                },
            ) = (node, saved)
            {
                *ratio = r.clamp(0.1, 0.9);
                apply_ratios(first, f);
                apply_ratios(second, s);
            }
        }
    }

    /// "Save current layout as launch configuration" (Warp Tab Configs): every
    /// tab's split tree + pane cwds as a Warp-compatible YAML file.
    fn save_current_layout(&mut self) {
        use runway_core::launch::{LaunchConfig, LaunchLayout, LaunchTab};
        fn layout(tab: &TabState, node: &crate::pane::PaneNode) -> LaunchLayout {
            match node {
                crate::pane::PaneNode::Leaf(id) => LaunchLayout::Pane {
                    cwd: tab
                        .pane(*id)
                        .and_then(|p| p.session.cwd.lock().unwrap().clone())
                        .map(std::path::PathBuf::from),
                    commands: Vec::new(),
                    focused: *id == tab.active_pane && tab.is_split(),
                },
                crate::pane::PaneNode::Split {
                    axis,
                    first,
                    second,
                    ..
                } => {
                    let vertical = *axis == Axis::Horizontal;
                    // flatten same-direction chains the way Warp nests them
                    let mut panes = Vec::new();
                    for child in [first, second] {
                        match layout(tab, child) {
                            LaunchLayout::Split {
                                vertical: v,
                                panes: inner,
                            } if v == vertical => panes.extend(inner),
                            other => panes.push(other),
                        }
                    }
                    LaunchLayout::split(vertical, panes)
                }
            }
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cfg = LaunchConfig {
            name: format!("Layout {}", utc_stamp(secs)),
            tabs: self
                .tabs
                .iter()
                .map(|t| LaunchTab {
                    title: t.custom_title.clone(),
                    layout: layout(t, &t.root),
                })
                .collect(),
            active_tab: self.active,
        };
        match runway_core::launch::save(&cfg) {
            Ok(path) => {
                self.launch_configs = runway_core::launch::load_all();
                let shown = editor::shorten_home(&path.to_string_lossy());
                self.toast(format!("Saved launch configuration to {shown}"));
            }
            Err(e) => self.toast(format!("Could not save launch configuration: {e}")),
        }
    }

    /// Transient status message at the bottom of the window.
    fn toast(&mut self, msg: String) {
        log::info!("toast: {msg}");
        self.toast = Some((msg, Instant::now()));
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Realize a saved pane tree inside the region held by `pane`.
    fn build_saved(&mut self, layout: &runway_core::session_state::SavedLayout, pane: PaneId) {
        use runway_core::session_state::{SavedAxis, SavedLayout};
        match layout {
            SavedLayout::Pane { focused, .. } => {
                if *focused && let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = pane;
                }
            }
            SavedLayout::Split {
                axis,
                first,
                second,
                ..
            } => {
                fn leaf_cwd(l: &SavedLayout) -> Option<std::path::PathBuf> {
                    match l {
                        SavedLayout::Pane { cwd, .. } => cwd.clone().map(std::path::PathBuf::from),
                        SavedLayout::Split { first, .. } => leaf_cwd(first),
                    }
                    .filter(|p| p.is_dir())
                }
                let axis = match axis {
                    SavedAxis::Horizontal => Axis::Horizontal,
                    SavedAxis::Vertical => Axis::Vertical,
                };
                let new_id = self.split_pane(pane, axis, leaf_cwd(second));
                self.build_saved(first, pane);
                self.build_saved(second, new_id);
            }
        }
    }

    fn split_active(&mut self, axis: Axis) {
        if self.tabs.is_empty() {
            return;
        }
        let dir = self.start_dir();
        let session = self.spawn_session(60, 20, dir); // resized on next layout
        let tab = &mut self.tabs[self.active];
        tab.zoomed = None;
        let target = tab.active_pane;
        let new_id = session.id;
        if tab.root.split(target, new_id, axis) {
            tab.panes.push(PaneState::new(session));
            tab.active_pane = new_id;
        }
        self.save_session();
    }

    fn close_pane(&mut self, pane: PaneId, event_loop: &ActiveEventLoop) {
        let Some(tab_idx) = self
            .tabs
            .iter()
            .position(|t| t.panes.iter().any(|p| p.session.id == pane))
        else {
            return;
        };
        let tab = &mut self.tabs[tab_idx];
        if tab.panes.len() <= 1 {
            // remember the cwd for ⇧⌘T (same source as save_session)
            let cwd = tab.active().and_then(|p| {
                p.session
                    .cwd
                    .lock()
                    .unwrap()
                    .clone()
                    .or_else(|| p.last_git_cwd.clone())
            });
            self.closed_tabs.push(cwd.map(std::path::PathBuf::from));
            if self.closed_tabs.len() > 10 {
                self.closed_tabs.remove(0);
            }
            self.tabs.remove(tab_idx);
            if self.tabs.is_empty() {
                event_loop.exit();
                return;
            }
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len() - 1;
            }
            return;
        }
        tab.zoomed = None;
        tab.root.remove(pane);
        tab.panes.retain(|p| p.session.id != pane);
        if tab.active_pane == pane {
            tab.active_pane = tab.root.leaves()[0];
        }
        self.save_session();
    }

    fn request_git(&mut self) {
        let mut changed = false;
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        for pane in &mut tab.panes {
            let cwd = pane.session.cwd.lock().unwrap().clone();
            if let Some(cwd) = cwd
                && pane.last_git_cwd.as_deref() != Some(cwd.as_str())
            {
                pane.last_git_cwd = Some(cwd.clone());
                if let Some(worker) = &self.git {
                    worker.request(pane.session.id, cwd.into());
                }
                changed = true;
            }
        }
        if changed {
            self.save_session();
        }
    }

    fn blink_on(&self) -> bool {
        (self.blink_epoch.elapsed().as_millis() / BLINK_INTERVAL.as_millis()).is_multiple_of(2)
    }

    /// Layout the active tab's panes within the content area.
    fn pane_rects(&self, w: f32, h: f32) -> (Vec<PaneRect>, Vec<HandleRect>) {
        let mut panes = Vec::new();
        let mut handles = Vec::new();
        let Some(tab) = self.tabs.get(self.active) else {
            return (panes, handles);
        };
        // ⇧⌘⏎ zoom: the zoomed pane takes the whole content rect (no handles
        // → no drag targets); hidden panes keep streaming, just not laid out
        if let Some(z) = tab.zoomed
            && tab.is_split()
            && tab.panes.iter().any(|p| p.session.id == z)
        {
            panes.push(PaneRect {
                id: z,
                rect: [0.0, TAB_BAR_H, w, h - TAB_BAR_H],
            });
            return (panes, handles);
        }
        if tab.is_split() {
            let area = [
                SIDE_PAD,
                TAB_BAR_H + SIDE_PAD,
                w - SIDE_PAD * 2.0,
                h - TAB_BAR_H - SIDE_PAD * 2.0,
            ];
            tab.root.layout(area, &mut panes, &mut handles, vec![]);
        } else {
            panes.push(PaneRect {
                id: tab.active_pane,
                rect: [0.0, TAB_BAR_H, w, h - TAB_BAR_H],
            });
        }
        (panes, handles)
    }

    fn build_scene(&mut self, w: f32, h: f32) {
        let theme = self.theme.clone();
        let blink = self.blink_on() || !self.config.cursor_blink;
        let cell_w = self.cell_w;
        let font_size = self.font_size;
        let line_h = self.line_h;
        let ligatures = self.config.ligatures;
        let cursor_style = self.config.cursor_style;
        let prompt_mode = self.config.prompt;
        let input_top = self.config.input_position == runway_core::InputPosition::Top;
        let autosuggest = self.config.autosuggestions;
        let side_pad = self.config.padding();
        let bell_flash = self.bell_flash.is_some_and(|t| t.elapsed() < BELL_FLASH);
        let settings_open = self.settings.open;
        let active_tab_idx = self.active;
        let (pane_rects, handles) = self.pane_rects(w, h);

        let shell_label = runway_core::shell_integration::shell_name(self.config.shell.as_deref());
        let tab_infos: Vec<tabbar::TabInfo> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| tabbar::TabInfo {
                title: t.display_title(),
                panes: if t.is_split() { t.panes.len() } else { 1 },
                shell_label: shell_label.clone(),
                activity: t.activity,
                color: t.color.map(|c| context_menu::tab_color(&theme, c)),
                sync: t.sync_inputs && t.is_split(),
                editing: self
                    .rename
                    .as_ref()
                    .filter(|(ri, _)| *ri == i)
                    .map(|(_, s)| s.clone()),
                editing_selected: self.rename_fresh,
            })
            .collect();
        // right-edge hint (design 01 "⌘K palette" / design 02 "⌃⇧D split"
        // pill), using whatever chord the keymap resolves today
        let split_view = self.tabs.get(active_tab_idx).is_some_and(|t| t.is_split());
        let hint_actions: &[&str] = if split_view {
            &["split_vertical_alt", "split_vertical"]
        } else {
            &["palette"]
        };
        let tab_hint = hint_actions
            .iter()
            .find_map(|a| {
                self.keymap
                    .bindings_for(a)
                    .first()
                    .map(|b| b.chips().concat())
            })
            .map(|chord| format!("{chord} {}", if split_view { "split" } else { "palette" }))
            .unwrap_or_default();
        let confirm = self.confirm.as_ref().map(|c| (c.scope, c.commands.clone()));
        let toast = self
            .toast
            .as_ref()
            .filter(|(_, t)| t.elapsed() < TOAST_TTL)
            .map(|(m, _)| m.clone());
        let filter_editing = self.filter_editing;
        let shell_env = self.shell_env.clone();
        let error_underline = self.config.error_underline;

        let Self {
            scene,
            text,
            tabs,
            completion,
            palette,
            palette_results,
            palette_row_hits,
            settings: settings_state,
            config: config_ref,
            keymap: keymap_ref,
            search: search_state,
            ..
        } = self;
        let Some(engine) = text.as_mut() else { return };
        let ed_char_w = engine.measure(
            "0",
            editor::FONT_SIZE,
            runway_renderer::mono_family(),
            runway_renderer::glyphon::Weight::NORMAL,
        );
        let mut active_ed_cols = 80usize;

        scene.clear();
        let mut hits = Vec::new();
        let mut focus_hits = Vec::new(); // lowest priority, appended last
        let mut any_running = false;
        let mut editor_active = false;
        let mut editor_anchor = (0.0f32, 0.0f32);
        let mut editor_zone = [0.0f32; 4];
        let mut ime_spot: Option<(f32, f32)> = None;
        let completion_ghost = completion.ghost.clone();

        // ── layer 0: chrome ──
        tabbar::render(
            scene.layer(0),
            engine,
            &theme,
            &tab_infos,
            &tab_hint,
            split_view,
            active_tab_idx,
            w,
            blink,
            &mut hits,
        );

        let Some(tab) = tabs.get_mut(active_tab_idx) else {
            self.hits = hits;
            return;
        };
        let split = tab.effective_split();

        if split {
            // gutter fill
            scene
                .layer(0)
                .rect([0.0, TAB_BAR_H], [w, h - TAB_BAR_H], theme.bg_gutter, 0.0);
        }

        // resize handles (drawn under panes is fine — gaps show gutter)
        for handle in &handles {
            let [hx, hy, hw, hh] = handle.rect;
            let (gx, gy, gw, gh) = match handle.axis {
                Axis::Horizontal => (hx + hw / 2.0 - 1.5, hy + hh / 2.0 - 19.0, 3.0, 38.0),
                Axis::Vertical => (hx + hw / 2.0 - 19.0, hy + hh / 2.0 - 1.5, 38.0, 3.0),
            };
            scene.layer(0).rect(
                [gx, gy],
                [gw, gh],
                runway_core::Color::rgba(1.0, 1.0, 1.0, 0.16),
                2.0,
            );
            hits.push(Hit {
                rect: handle.rect,
                target: ClickTarget::HandleDrag(handle.path.clone()),
            });
        }

        for pr in &pane_rects {
            let Some(pane) = tab.panes.iter_mut().find(|p| p.session.id == pr.id) else {
                continue;
            };
            let is_active_pane = pr.id == tab.active_pane;
            let editor_vis = Self::editor_visible(pane);
            if is_active_pane {
                editor_active = editor_vis;
            }
            // soft-wrap width of this pane's command editor
            let editor_area_w = if split {
                pr.rect[2] - PANE_PAD * 2.0
            } else {
                w - side_pad * 2.0
            };
            let ed_cols =
                editor::input_cols(editor_area_w, cell_w, ed_char_w, pane.editor.vim_enabled());
            if is_active_pane {
                active_ed_cols = ed_cols;
            }

            let grid = {
                // inline grid_for (self is destructured)
                let rect = pr.rect;
                let (x, y, pw, ph) = if split {
                    (
                        rect[0] + PANE_PAD,
                        rect[1] + PANE_PAD,
                        rect[2] - PANE_PAD * 2.0,
                        rect[3] - PANE_PAD * 2.0,
                    )
                } else {
                    (
                        rect[0] + side_pad,
                        rect[1] + TOP_PAD,
                        rect[2] - side_pad * 2.0,
                        rect[3] - TOP_PAD,
                    )
                };
                let editor_h = if !editor_vis {
                    if split { PANE_PAD } else { SIDE_PAD }
                } else {
                    editor::editor_height(&pane.editor.buffer, split, prompt_mode, ed_cols)
                };
                let usable_w = (pw - terminal::BLOCK_INDENT).max(cell_w);
                let usable_h = (ph - editor_h).max(line_h);
                // input at the top: the grid starts below the editor zone
                let y = if editor_vis && input_top {
                    y + editor_h - (if split { PANE_PAD } else { TOP_PAD - 4.0 })
                } else {
                    y
                };
                GridLayout {
                    x,
                    y,
                    right: x + pw,
                    cell_w,
                    line_h,
                    font_size,
                    cols: (usable_w / cell_w).floor().max(2.0) as usize,
                    rows: (usable_h / line_h).floor().max(2.0) as usize,
                    ligatures,
                    cursor_style,
                    editor_owns_cursor: editor_vis,
                }
            };

            pane.session.resize(
                GridSize {
                    cols: grid.cols,
                    rows: grid.rows,
                },
                (cell_w.round() as u16, line_h.round() as u16),
            );

            // pane chrome (design 02)
            if split {
                let layer0 = scene.layer(0);
                if is_active_pane {
                    layer0.shadow(
                        [pr.rect[0], pr.rect[1]],
                        [pr.rect[2], pr.rect[3]],
                        theme.accent.with_alpha(0.10),
                        PANE_RADIUS,
                        4.0,
                    );
                    layer0.bordered(
                        [pr.rect[0], pr.rect[1]],
                        [pr.rect[2], pr.rect[3]],
                        theme.bg,
                        theme.accent,
                        PANE_RADIUS,
                        2.0,
                    );
                } else {
                    layer0.bordered(
                        [pr.rect[0], pr.rect[1]],
                        [pr.rect[2], pr.rect[3]],
                        theme.bg,
                        theme.pane_border_inactive,
                        PANE_RADIUS,
                        1.0,
                    );
                }
                focus_hits.push(Hit {
                    rect: pr.rect,
                    target: ClickTarget::PaneFocus(pr.id),
                });
            }

            // content
            let layer = scene.layer(1);
            let text_start = layer.texts.len();
            let search_ctx = if search_state.open && is_active_pane && !filter_editing {
                search_state.matcher().map(|matcher| terminal::SearchCtx {
                    matcher,
                    current: search_state.current_anchor(),
                })
            } else {
                None
            };
            let out = terminal::render(
                layer,
                engine,
                &theme,
                &pane.session,
                &grid,
                &mut pane.view_offset,
                // shell-prompt mode keeps the shell's PS1 rows on screen
                editor_vis && prompt_mode != runway_core::PromptMode::Shell,
                search_ctx.as_ref(),
                if is_active_pane {
                    pane.selected_block
                } else {
                    None
                },
                pane.filter.as_ref(),
            );
            if pane.view_offset > 0 && out.display_len > pane.last_display_len {
                pane.view_offset += out.display_len - pane.last_display_len;
            }
            pane.last_display_len = out.display_len;
            pane.rows = out.rows;
            pane.last_grid = Some(grid);
            pane.alt_screen = out.alt_screen;
            pane.has_selection = {
                // cheap mirror: render computed to_range under the same lock
                pane.session.term.lock().selection.is_some()
            };
            any_running |= out.any_running;
            for hb in out.hits {
                hits.push(Hit {
                    rect: hb.rect,
                    target: ClickTarget::Block(pr.id, hb.block),
                });
            }
            for hb in out.chevron_hits {
                hits.push(Hit {
                    rect: hb.rect,
                    target: ClickTarget::BlockChevron(pr.id, hb.block),
                });
            }
            for hb in out.copy_hits {
                hits.push(Hit {
                    rect: hb.rect,
                    target: ClickTarget::BlockCopy(pr.id, hb.block),
                });
            }
            for (rect, id) in &out.bookmark_hits {
                hits.push(Hit {
                    rect: *rect,
                    target: ClickTarget::JumpToBlock(pr.id, *id),
                });
            }
            pane.url_hits = out.url_hits;
            let sticky_block = out.sticky_block;

            // Warp-style boot placeholder until the shell's first byte
            if !pane
                .session
                .saw_output
                .load(std::sync::atomic::Ordering::Acquire)
            {
                let shell = runway_core::shell_integration::shell_name(config_ref.shell.as_deref());
                let label = format!("Starting {shell}\u{2026}");
                let buf = {
                    use runway_renderer::glyphon::{
                        Attrs, Buffer, Family, Metrics, Shaping, Weight,
                    };
                    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(13.0, line_h));
                    b.set_text(
                        &mut engine.font_system,
                        &label,
                        &Attrs::new()
                            .family(Family::Name(runway_renderer::mono_family()))
                            .weight(Weight::MEDIUM),
                        Shaping::Advanced,
                        None,
                    );
                    b
                };
                layer.text(buf, grid.x, grid.y, theme.syntax.string);
            }

            pane.last_exit = pane
                .session
                .blocks
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|b| b.exit_code());

            if editor_vis {
                let eh = editor::editor_height(&pane.editor.buffer, split, prompt_mode, ed_cols);
                let area = match (split, input_top) {
                    (true, false) => [
                        pr.rect[0] + PANE_PAD,
                        pr.rect[1] + pr.rect[3] - eh,
                        pr.rect[2] - PANE_PAD * 2.0,
                        eh,
                    ],
                    (true, true) => [
                        pr.rect[0] + PANE_PAD,
                        pr.rect[1] + PANE_PAD - 4.0,
                        pr.rect[2] - PANE_PAD * 2.0,
                        eh,
                    ],
                    (false, false) => [side_pad, h - eh, w - side_pad * 2.0, eh],
                    (false, true) => [side_pad, pr.rect[1] + 4.0, w - side_pad * 2.0, eh],
                };
                let anchor = editor::render(
                    layer,
                    engine,
                    &theme,
                    config_ref,
                    pane,
                    area,
                    cell_w,
                    blink && is_active_pane,
                    is_active_pane,
                    split,
                    if is_active_pane && autosuggest {
                        &completion_ghost
                    } else {
                        ""
                    },
                    shell_env.as_deref().filter(|_| error_underline),
                    if is_active_pane {
                        pane.correction.as_deref()
                    } else {
                        None
                    },
                    ed_cols,
                );
                if is_active_pane {
                    editor_anchor = anchor.0;
                    editor_zone = area;
                    ime_spot = Some(anchor.1);
                }
                hits.push(Hit {
                    rect: area,
                    target: ClickTarget::Editor(pr.id),
                });
            } else if is_active_pane {
                // raw mode: the program's cursor cell
                let (line, col) = {
                    let term = pane.session.term.lock();
                    let p = term.grid().cursor.point;
                    (p.line.0, p.column.0)
                };
                let scrolled = pane
                    .session
                    .scrolled
                    .load(std::sync::atomic::Ordering::Acquire);
                let abs = scrolled as i64 + line as i64;
                ime_spot = pane.rows.iter().find_map(|r| match r.kind {
                    terminal::RowKind::Grid { abs: a, content_x } if a as i64 == abs => {
                        Some((content_x + col as f32 * cell_w, r.y))
                    }
                    _ => None,
                });
            }

            // clip pane text to its rect
            if split {
                for t in &mut layer.texts[text_start..] {
                    t.bounds = Some(TextBounds {
                        left: (pr.rect[0] + 2.0) as i32,
                        top: (pr.rect[1] + 2.0) as i32,
                        right: (pr.rect[0] + pr.rect[2] - 2.0) as i32,
                        bottom: (pr.rect[1] + pr.rect[3] - 2.0) as i32,
                    });
                }
            }

            // sticky command header (active pane only; layer 2 covers row text)
            if is_active_pane
                && let Some(sticky) = sticky_block
                && let Some(rect) = terminal::render_sticky_header(
                    scene.layer(2),
                    engine,
                    &theme,
                    &grid,
                    &pane.session,
                    sticky,
                )
            {
                hits.push(Hit {
                    rect,
                    target: ClickTarget::JumpToBlock(pr.id, sticky),
                });
            }

            // inactive pane dim (design: 62% opacity)
            if split && !is_active_pane {
                scene.layer(2).rect(
                    [pr.rect[0], pr.rect[1]],
                    [pr.rect[2], pr.rect[3]],
                    theme.bg_gutter.with_alpha(0.38),
                    PANE_RADIUS,
                );
            }
        }

        // visual bell: brief full-window flash
        if bell_flash {
            scene
                .layer(2)
                .rect([0.0, 0.0], [w, h], theme.fg.with_alpha(0.10), 0.0);
        }

        // ── layer 2: completion popup (above editor) ──
        if editor_active && completion.open && !palette.open {
            let prefix_len = tabs
                .get(active_tab_idx)
                .and_then(|t| t.active())
                .map(|p| {
                    let (_, token) =
                        runway_core::completion::current_token(&p.editor.buffer, p.editor.cursor);
                    token.len()
                })
                .unwrap_or(0);
            completion_view::render(
                scene.layer(2),
                engine,
                &theme,
                completion,
                editor_anchor,
                editor_zone,
                h,
                prefix_len,
            );
        }

        // ── layer 2: IME composition (preedit) at the cursor ──
        if !self.preedit.is_empty()
            && let Some((px, py)) = ime_spot
        {
            let size = if editor_active {
                editor::FONT_SIZE
            } else {
                font_size
            };
            let tw = engine.measure(
                &self.preedit,
                size,
                runway_renderer::mono_family(),
                runway_renderer::glyphon::Weight::NORMAL,
            );
            let layer2 = scene.layer(2);
            layer2.rect(
                [px - 1.0, py - 1.0],
                [tw + 2.0, line_h.max(20.0)],
                theme.bg,
                2.0,
            );
            layer2.rect([px, py + size + 3.0], [tw, 1.5], theme.accent, 0.0);
            let buf = {
                use runway_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping};
                let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.4));
                b.set_text(
                    &mut engine.font_system,
                    &self.preedit,
                    &Attrs::new().family(Family::Name(runway_renderer::mono_family())),
                    Shaping::Advanced,
                    None,
                );
                b
            };
            layer2.text(buf, px, py, theme.fg);
        }
        self.ime_spot = ime_spot;

        // ── layer 2: search bar ──
        if search_state.open && !settings_open {
            let filter = tabs
                .get(active_tab_idx)
                .and_then(|t| t.active())
                .and_then(|p| p.filter.clone());
            let chips = match (filter_editing, filter) {
                (true, Some((_, q))) => search::render(
                    scene.layer(2),
                    engine,
                    &theme,
                    &q,
                    "Enter keep · Esc clear",
                    Some("filter"),
                    w,
                    blink,
                    None,
                ),
                _ => search::render(
                    scene.layer(2),
                    engine,
                    &theme,
                    &search_state.query,
                    &search::counter(search_state),
                    None,
                    w,
                    blink,
                    Some((search_state.case_sensitive, search_state.regex)),
                ),
            };
            self.search_chips = chips;
        } else {
            self.search_chips.clear();
        }

        // ── layer 3: settings overlay ──
        if settings_open {
            // the overlay covers the terminal: its controls must win hit-testing
            hits.clear();
            settings_state.content_h = settings::render(
                scene.layer(3),
                engine,
                &theme,
                settings_state,
                config_ref,
                keymap_ref,
                w,
                h,
                &mut hits,
            );
        }

        // ── layer 4: command palette ──
        palette_row_hits.clear();
        if palette.open {
            palette::render(
                scene.layer(4),
                engine,
                &theme,
                palette,
                palette_results,
                w,
                h,
                blink,
                &mut hits,
                palette_row_hits,
            );
        }

        // ── layer 5: context menu ──
        if self.context_menu.open {
            self.context_menu_rects =
                context_menu::render(scene.layer(5), engine, &theme, &self.context_menu, w, h);
        } else {
            self.context_menu_rects.clear();
        }

        // ── layer 6: toast + close/quit warning (modal: eats every click) ──
        if let Some(msg) = &toast {
            crate::views::confirm::toast(scene.layer(6), engine, &theme, msg, w, h);
        }
        if let Some((scope, commands)) = &confirm {
            hits.clear();
            let (title, label) = match scope {
                CloseScope::Pane(_) => ("Close pane?", "Close pane"),
                CloseScope::Tab(_) => ("Close tab?", "Close tab"),
                CloseScope::OtherTabs(_) => ("Close other tabs?", "Close tabs"),
                CloseScope::Window => ("Quit Runway?", "Quit"),
            };
            self.confirm_rects = crate::views::confirm::render(
                scene.layer(6),
                engine,
                &theme,
                title,
                commands,
                label,
                w,
                h,
            );
        }

        hits.extend(focus_hits);
        self.hits = hits;
        self.any_running = any_running;
        self.editor_active = editor_active;
        self.editor_anchor = editor_anchor;
        self.editor_cols = active_ed_cols;
    }

    fn redraw(&mut self, window: &Window) {
        let scale = window.scale_factor() as f32;
        if self.gpu.is_none() {
            return;
        }
        self.request_git();

        let (pw, ph) = {
            let gpu = self.gpu.as_ref().unwrap();
            (gpu.config.width, gpu.config.height)
        };
        let (lw, lh) = (pw as f32 / scale, ph as f32 / scale);

        self.build_scene(lw, lh);
        if let Some(spot) = self.ime_spot
            && self.ime_area_sent != Some(spot)
        {
            self.ime_area_sent = Some(spot);
            window.set_ime_cursor_area(
                winit::dpi::LogicalPosition::new(spot.0, spot.1),
                LogicalSize::new(self.cell_w, self.line_h),
            );
        }

        let mut all_quads = Vec::new();
        let mut ranges = Vec::new();
        for layer in &self.scene.layers {
            let start = all_quads.len() as u32;
            all_quads.extend_from_slice(&layer.quads);
            ranges.push(start..all_quads.len() as u32);
        }

        let (Some(gpu), Some(quads), Some(text)) = (&mut self.gpu, &mut self.quads, &mut self.text)
        else {
            return;
        };
        // prepared once; drawn first in every pass (main + capture)
        if let Some(img) = self.bg_image.as_mut()
            && img.has_image()
        {
            img.prepare(&gpu.queue, [pw as f32, ph as f32], self.bg_image_opacity);
        }
        let bg_image = self.bg_image.as_ref().filter(|p| p.has_image());

        quads.prepare(&gpu.device, &gpu.queue, [lw, lh], &all_quads);

        let text_layers: Vec<Vec<TextArea<'_>>> = self
            .scene
            .layers
            .iter()
            .map(|layer| {
                layer
                    .texts
                    .iter()
                    .map(|t| TextArea {
                        buffer: t.buffer.as_ref(),
                        left: t.left * scale,
                        top: t.top * scale,
                        scale,
                        bounds: t
                            .bounds
                            .map(|b| TextBounds {
                                left: (b.left as f32 * scale) as i32,
                                top: (b.top as f32 * scale) as i32,
                                right: (b.right as f32 * scale) as i32,
                                bottom: (b.bottom as f32 * scale) as i32,
                            })
                            .unwrap_or(TextBounds {
                                left: 0,
                                top: 0,
                                right: pw as i32,
                                bottom: ph as i32,
                            }),
                        default_color: t.color,
                        custom_glyphs: &[],
                    })
                    .collect()
            })
            .collect();
        text.prepare_layers(&gpu.device, &gpu.queue, (pw, ph), text_layers);

        let Some(frame) = gpu.acquire() else { return };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(to_wgpu(self.theme.bg)),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(img) = bg_image {
                img.render(&mut pass);
            }
            for (i, range) in ranges.iter().cloned().enumerate() {
                quads.render_range(&mut pass, range);
                text.render_layer(i, &mut pass);
            }
        }
        let capture_ranges = ranges;
        gpu.queue.submit([encoder.finish()]);
        if let Some(path) = self.capture.take() {
            crate::capture::capture_frame(
                gpu,
                quads,
                text,
                bg_image,
                &capture_ranges,
                self.theme.bg,
                &path,
            );
        }
        window.pre_present_notify();
        frame.present();
        text.trim();
    }

    fn handle_action(&mut self, action: AppAction, event_loop: &ActiveEventLoop) {
        match action {
            AppAction::NewTab => self.spawn_tab(),
            AppAction::ClosePane => {
                if let Some(tab) = self.tabs.get(self.active) {
                    self.request_close(CloseScope::Pane(tab.active_pane), event_loop);
                }
            }
            AppAction::CloseWindow | AppAction::Quit => {
                self.request_close(CloseScope::Window, event_loop);
            }
            AppAction::CloseOtherTabs => {
                self.request_close(CloseScope::OtherTabs(self.active), event_loop);
            }
            AppAction::NewWindow => self.open_new_window(),
            AppAction::Minimize => {
                if let Some(w) = &self.window {
                    w.set_minimized(true);
                }
            }
            AppAction::HideApp => {
                #[cfg(target_os = "macos")]
                {
                    use winit::platform::macos::ActiveEventLoopExtMacOS;
                    event_loop.hide_application();
                }
                #[cfg(not(target_os = "macos"))]
                if let Some(w) = &self.window {
                    w.set_minimized(true);
                }
            }
            AppAction::ToggleFullscreen => {
                if let Some(w) = &self.window {
                    let full = w.fullscreen().is_some();
                    w.set_fullscreen(
                        (!full).then_some(winit::window::Fullscreen::Borderless(None)),
                    );
                }
            }
            AppAction::RenameTab => {
                self.start_rename(self.active);
            }
            AppAction::ToggleSyncInputs => {
                if let Some(t) = self.tabs.get_mut(self.active) {
                    t.sync_inputs = !t.sync_inputs;
                }
            }
            AppAction::FilterBlock => {
                if let Some((pane_id, block)) = self.selected_or_last_block() {
                    self.toggle_block_filter(pane_id, block);
                }
            }
            AppAction::SelectAll => {
                let editor_active = self.editor_active;
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    if editor_active && !pane.editor.buffer.is_empty() {
                        pane.editor.select_all();
                    } else {
                        // select the whole scrollback + screen
                        let mut term = pane.session.term.lock();
                        use runway_pty::alacritty_terminal::grid::Dimensions;
                        let top = ALine(-(term.grid().history_size() as i32));
                        let bottom = ALine(term.grid().screen_lines() as i32 - 1);
                        let right = ACol(term.grid().columns().saturating_sub(1));
                        let mut sel = Selection::new(
                            SelectionType::Simple,
                            APoint::new(top, ACol(0)),
                            ASide::Left,
                        );
                        sel.update(APoint::new(bottom, right), ASide::Right);
                        term.selection = Some(sel);
                        drop(term);
                        pane.has_selection = true;
                    }
                }
            }
            AppAction::SaveLaunchConfig => self.save_current_layout(),
            AppAction::TabNext => {
                if !self.tabs.is_empty() {
                    self.switch_tab((self.active + 1) % self.tabs.len());
                }
            }
            AppAction::TabPrev => {
                if !self.tabs.is_empty() {
                    self.switch_tab((self.active + self.tabs.len() - 1) % self.tabs.len());
                }
            }
            AppAction::TabSelect(n) => {
                if !self.tabs.is_empty() {
                    // Warp: ⌘9 = last tab; out-of-range numbers are no-ops
                    let idx = if n == 9 {
                        self.tabs.len() - 1
                    } else {
                        n as usize - 1
                    };
                    if idx < self.tabs.len() && idx != self.active {
                        self.switch_tab(idx);
                    }
                }
            }
            AppAction::MoveTabLeft | AppAction::MoveTabRight => {
                let n = self.tabs.len();
                let target = if action == AppAction::MoveTabLeft {
                    self.active.checked_sub(1)
                } else {
                    (self.active + 1 < n).then_some(self.active + 1)
                };
                if let Some(t) = target {
                    self.tabs.swap(self.active, t);
                    self.active = t;
                    self.save_session();
                }
            }
            AppAction::ReopenClosedTab => {
                if let Some(cwd) = self.closed_tabs.pop() {
                    self.spawn_tab_at(cwd);
                }
            }
            AppAction::ScrollUp | AppAction::ScrollDown => {
                let delta = if action == AppAction::ScrollUp {
                    10i64
                } else {
                    -10
                };
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && let Some(pane) = tab.active_mut()
                {
                    pane.view_offset = (pane.view_offset as i64 + delta).max(0) as usize;
                }
            }
            AppAction::SplitRight => self.split_active(Axis::Horizontal),
            AppAction::SplitDown => self.split_active(Axis::Vertical),
            AppAction::TogglePalette => {
                if self.palette.open {
                    self.palette.open = false;
                } else {
                    self.open_palette(crate::views::palette::PaletteMode::Commands);
                }
            }
            AppAction::Workflows => {
                if self.palette.open {
                    self.palette.open = false;
                } else {
                    self.open_palette(crate::views::palette::PaletteMode::Workflows);
                }
            }
            AppAction::OpenSettings => {
                self.settings.open = !self.settings.open;
            }
            AppAction::BookmarkToggle => {
                // selected block wins; else the most recent finished block
                if let Some((pane_id, id)) = self.selected_or_last_block()
                    && let Some(pane) = self.tabs.get(self.active).and_then(|t| t.pane(pane_id))
                {
                    pane.session.blocks.lock().unwrap().toggle_bookmark(id);
                }
            }
            AppAction::BookmarkPrev => self.jump_bookmark(-1),
            AppAction::BookmarkNext => self.jump_bookmark(1),
            AppAction::Copy => {
                let editor_sel = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.active())
                    .filter(|_| self.editor_active)
                    .and_then(|p| p.editor.selected_text().map(str::to_string));
                if let Some(text) = editor_sel {
                    self.copy_to_clipboard(text);
                    return;
                }
                if let Some(text) = self.selection_text() {
                    self.copy_to_clipboard(text);
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                // no selection: copy the last finished block's output
                let text = self
                    .tabs
                    .get(self.active)
                    .and_then(|t| t.active())
                    .and_then(|p| {
                        let id = p
                            .session
                            .blocks
                            .lock()
                            .unwrap()
                            .iter()
                            .rev()
                            .find(|b| b.output_range().is_some())
                            .map(|b| b.id)?;
                        terminal::block_output_text(&p.session, id)
                    });
                if let Some(text) = text {
                    self.copy_to_clipboard(text);
                }
            }
            AppAction::Paste => self.paste_from_clipboard(),
            AppAction::Search => {
                if self.search.open {
                    self.search.close();
                } else {
                    self.search.open = true;
                    self.refresh_search(true);
                    self.blink_epoch = Instant::now();
                }
            }
            AppAction::FocusLeft
            | AppAction::FocusRight
            | AppAction::FocusUp
            | AppAction::FocusDown => {
                let dir = match action {
                    AppAction::FocusLeft => (-1.0, 0.0),
                    AppAction::FocusRight => (1.0, 0.0),
                    AppAction::FocusUp => (0.0, -1.0),
                    _ => (0.0, 1.0),
                };
                // navigating unzooms first, then moves within the real layout
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.zoomed = None;
                }
                let (lw, lh) = self.logical_size();
                let (rects, _) = self.pane_rects(lw, lh);
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && let Some(next) = focus_neighbor(&rects, tab.active_pane, dir)
                {
                    if let Some(prev) = tab.active_mut() {
                        prev.selected_block = None;
                    }
                    tab.active_pane = next;
                }
            }
            AppAction::ClearBlocks => {
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
                    && !pane.alt_screen
                {
                    pane.session.clear_screen_and_blocks();
                    pane.view_offset = 0;
                    pane.last_display_len = 0;
                    pane.selected_block = None;
                    self.search.matches.clear();
                }
            }
            AppAction::FontIncrease | AppAction::FontDecrease | AppAction::FontReset => {
                self.config.font_size = match action {
                    AppAction::FontIncrease => (self.config.font_size + 1.0).clamp(8.0, 28.0),
                    AppAction::FontDecrease => (self.config.font_size - 1.0).clamp(8.0, 28.0),
                    _ => DEFAULT_FONT_SIZE,
                };
                self.save_config();
                self.apply_config();
            }
            AppAction::FocusInput => {
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    pane.view_offset = 0;
                    pane.selected_block = None;
                }
                self.blink_epoch = Instant::now();
            }
            AppAction::ScrollPageUp
            | AppAction::ScrollPageDown
            | AppAction::ScrollTop
            | AppAction::ScrollBottom => {
                if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                    let rows = pane.last_grid.map(|g| g.rows).unwrap_or(24);
                    let max_off = pane.last_display_len.saturating_sub(rows);
                    pane.view_offset = match action {
                        AppAction::ScrollPageUp => (pane.view_offset + rows).min(max_off),
                        AppAction::ScrollPageDown => pane.view_offset.saturating_sub(rows),
                        AppAction::ScrollTop => max_off,
                        _ => 0,
                    };
                }
            }
            AppAction::OpenKeybindings => {
                self.settings
                    .open_page(crate::views::settings::Page::Keybindings);
            }
            AppAction::MaximizePane => {
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && tab.is_split()
                {
                    tab.zoomed = if tab.zoomed.is_some() {
                        None
                    } else {
                        Some(tab.active_pane)
                    };
                }
            }
            AppAction::ResizePaneLeft
            | AppAction::ResizePaneRight
            | AppAction::ResizePaneUp
            | AppAction::ResizePaneDown => {
                let (axis, delta) = match action {
                    AppAction::ResizePaneLeft => (Axis::Horizontal, -0.05),
                    AppAction::ResizePaneRight => (Axis::Horizontal, 0.05),
                    AppAction::ResizePaneUp => (Axis::Vertical, -0.05),
                    _ => (Axis::Vertical, 0.05),
                };
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && tab.zoomed.is_none()
                    && let Some(ancestors) = tab.root.ancestors_of(tab.active_pane)
                    && let Some((path, _, _)) =
                        ancestors.into_iter().rev().find(|(_, a, _)| *a == axis)
                {
                    tab.root.adjust_ratio(&path, delta);
                }
            }
            AppAction::SelectBlockPrev => self.select_block(-1),
            AppAction::SelectBlockNext => self.select_block(1),
            AppAction::CopyBlockCommand => {
                if let Some((pane_id, block)) = self.selected_or_last_block()
                    && let Some(cmd) = self.block_command(pane_id, block)
                {
                    self.copy_to_clipboard(cmd);
                }
            }
            AppAction::CopyBlockOutput => {
                if let Some((pane_id, block)) = self.selected_or_last_block() {
                    let text = self
                        .tabs
                        .get(self.active)
                        .and_then(|t| t.pane(pane_id))
                        .and_then(|p| terminal::block_output_text(&p.session, block));
                    if let Some(text) = text {
                        self.copy_to_clipboard(text);
                    }
                }
            }
            AppAction::Reinput => {
                if self.editor_active
                    && let Some((pane_id, block)) = self.selected_or_last_block()
                    && let Some(cmd) = self.block_command(pane_id, block)
                    && let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
                {
                    pane.editor.clear();
                    pane.editor.insert(&cmd);
                    pane.selected_block = None;
                    pane.view_offset = 0;
                    self.update_completions();
                    self.blink_epoch = Instant::now();
                }
            }
            AppAction::BlockMenu => {
                if let Some((pane_id, block)) = self.selected_or_last_block() {
                    let anchor = self
                        .tabs
                        .get(self.active)
                        .and_then(|t| t.pane(pane_id))
                        .and_then(|p| {
                            let y = p.rows.iter().find_map(|r| match r.kind {
                                crate::views::terminal::RowKind::Header(id) if id == block => {
                                    Some(r.y)
                                }
                                _ => None,
                            })?;
                            let g = p.last_grid?;
                            Some((g.x + 24.0, y + g.line_h))
                        });
                    match anchor {
                        Some(pos) => self.open_context_menu(pane_id, block, pos),
                        None => {
                            // header offscreen: bring it into view, anchor at
                            // the viewport top (rows cache refreshes next frame)
                            self.jump_to_block(pane_id, block);
                            let pos = self
                                .tabs
                                .get(self.active)
                                .and_then(|t| t.pane(pane_id))
                                .and_then(|p| p.last_grid)
                                .map(|g| (g.x + 24.0, g.y + g.line_h))
                                .unwrap_or(self.mouse);
                            self.open_context_menu(pane_id, block, pos);
                        }
                    }
                }
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Switch to tab `idx`, clearing transient state on the pane being left.
    fn switch_tab(&mut self, idx: usize) {
        if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
            pane.selected_block = None;
        }
        self.active = idx;
        self.tabs[idx].activity = false;
        self.save_session();
    }

    fn handle_settings_key(&mut self, event: &input::KeyPress, window: &Window) -> bool {
        use winit::keyboard::{Key, NamedKey};

        if self.settings.page == crate::views::settings::Page::Keybindings
            && self.handle_keybindings_key(event)
        {
            return true;
        }
        let action = input::app_action(event, self.modifiers, &self.keymap);
        if matches!(event.logical_key, Key::Named(NamedKey::Escape))
            || action == Some(AppAction::OpenSettings)
        {
            self.settings.open = false;
            return true;
        }
        match event.logical_key {
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowLeft) => {
                self.settings.move_page(-1);
                true
            }
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowRight) => {
                self.settings.move_page(1);
                true
            }
            Key::Named(NamedKey::Home) => {
                self.settings
                    .open_page(crate::views::settings::Page::General);
                true
            }
            Key::Named(NamedKey::End) => {
                self.settings
                    .open_page(crate::views::settings::Page::Themes);
                true
            }
            Key::Named(NamedKey::PageUp | NamedKey::PageDown) => {
                let direction = if matches!(event.logical_key, Key::Named(NamedKey::PageUp)) {
                    10.0
                } else {
                    -10.0
                };
                let viewport_h = window.inner_size().height as f32 / window.scale_factor() as f32
                    - crate::views::tabbar::TAB_BAR_H
                    - 80.0;
                self.settings.scroll_by(direction, viewport_h);
                true
            }
            _ => false,
        }
    }

    /// Keybindings page keys: while recording, the next chord becomes the
    /// action's shortcut (Esc cancels, ⌫ unbinds); otherwise typing filters.
    /// Returns true when consumed.
    fn handle_keybindings_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let mods = self.modifiers;
        if let Some(action) = self.settings.recording.clone() {
            let plain = !mods.control_key() && !mods.alt_key() && !mods.super_key();
            match &event.logical_key {
                Key::Named(NamedKey::Escape) if plain => {
                    self.settings.recording = None;
                    return true;
                }
                Key::Named(NamedKey::Backspace) if plain && !mods.shift_key() => {
                    self.settings.recording = None;
                    self.config
                        .keybindings
                        .insert(action.clone(), "none".into());
                    self.save_config();
                    self.apply_config();
                    self.toast(format!("{action} unbound"));
                    return true;
                }
                _ => {}
            }
            let key = match (&event.unmodified, &event.logical_key) {
                (Some(Key::Character(s)), Key::Character(_)) if mods.alt_key() => {
                    Some(s.to_lowercase())
                }
                _ => input::key_string(event),
            };
            let Some(key) = key else {
                return true; // a bare modifier: keep waiting
            };
            let function_key = key.len() > 1 && !matches!(key.as_str(), "space" | "enter" | "tab");
            if plain && !function_key {
                self.toast("Shortcuts need ⌘, ⌃ or ⌥ (or a function/navigation key)".into());
                return true;
            }
            let binding = runway_core::keybindings::Binding {
                ctrl: mods.control_key(),
                shift: mods.shift_key(),
                alt: mods.alt_key(),
                cmd: mods.super_key(),
                key,
            };
            self.settings.recording = None;
            let chord = binding.chord();
            // the chord moves: whoever had it keeps its other shortcuts
            let mut note = String::new();
            if let Some(other) = self.keymap.conflict(&binding, &action).map(str::to_string) {
                let rest = self
                    .keymap
                    .bindings_for(&other)
                    .into_iter()
                    .find(|b| **b != binding)
                    .map(|b| b.chord());
                self.config
                    .keybindings
                    .insert(other.clone(), rest.unwrap_or_else(|| "none".into()));
                note = format!(" (moved from {other})");
            }
            self.config.keybindings.insert(action.clone(), chord);
            self.save_config();
            self.apply_config();
            let shown = binding.chips().concat();
            self.toast(format!("{action} → {shown}{note}"));
            return true;
        }
        // type-to-filter
        if mods.super_key() || mods.control_key() {
            return false;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) if !self.settings.kb_filter.is_empty() => {
                self.settings.kb_filter.pop();
                self.settings.scroll = 0.0;
                true
            }
            Key::Named(NamedKey::Escape) if !self.settings.kb_filter.is_empty() => {
                self.settings.kb_filter.clear();
                true
            }
            Key::Named(NamedKey::Space) => {
                self.settings.kb_filter.push(' ');
                true
            }
            Key::Character(_) => match &event.text {
                Some(t) if !t.chars().any(char::is_control) => {
                    self.settings.kb_filter.push_str(t);
                    self.settings.scroll = 0.0;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn handle_editor_key(&mut self, key: EditorKey) {
        // an offered correction is accepted with → / Tab / End in an empty editor
        if matches!(key, EditorKey::Right | EditorKey::Complete | EditorKey::End)
            && let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
            && pane.editor.buffer.is_empty()
            && let Some(fix) = pane.correction.take()
        {
            pane.editor.insert(&fix);
            self.update_completions();
            self.blink_epoch = Instant::now();
            return;
        }
        // alias expansion (Warp): Space right after an alias in command
        // position replaces it with its definition
        if key == EditorKey::Insert(" ".into())
            && self.config.alias_expansion
            && let Some(env) = self.shell_env.clone()
            && let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
            && pane.editor.selection().is_none()
            && !pane.editor.in_placeholder_mode()
            && let Some((range, expansion)) =
                env.expand_alias_at(&pane.editor.buffer, pane.editor.cursor)
        {
            pane.editor.replace_range(range, &expansion);
        }
        // completion popup intercepts navigation keys
        if self.completion.open {
            match key {
                EditorKey::HistoryUp => {
                    self.completion.selected = self.completion.selected.saturating_sub(1);
                    self.blink_epoch = Instant::now();
                    return;
                }
                EditorKey::HistoryDown => {
                    if self.completion.selected + 1 < self.completion.items.len() {
                        self.completion.selected += 1;
                    }
                    self.blink_epoch = Instant::now();
                    return;
                }
                EditorKey::Dismiss => {
                    self.completion.close();
                    return;
                }
                _ => {}
            }
        }
        if key == EditorKey::Complete {
            // placeholder cycling wins over completion accept
            let cycled = self
                .tabs
                .get_mut(self.active)
                .and_then(|t| t.active_mut())
                .map(|p| p.editor.cycle_placeholder())
                .unwrap_or(false);
            if !cycled {
                self.accept_completion();
            }
            self.blink_epoch = Instant::now();
            return;
        }
        if key == EditorKey::HistorySearch {
            self.open_palette(crate::views::palette::PaletteMode::History);
            return;
        }
        if key == EditorKey::AcceptGhostWord {
            self.accept_ghost_word();
            self.blink_epoch = Instant::now();
            return;
        }

        let autocomplete_pairs = self.config.autocomplete_pairs;
        let mut submitted: Option<(PaneId, String)> = None;
        let mut signal: Option<(PaneId, &'static [u8])> = None;
        let mut cut: Option<String> = None;
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return;
        };
        let mut edited = true;
        match key {
            EditorKey::Cut => cut = pane.editor.cut_selection(),
            EditorKey::SelectLeft => pane.editor.extend(crate::state::EditorState::mv_left),
            EditorKey::SelectRight => pane.editor.extend(crate::state::EditorState::mv_right),
            EditorKey::SelectWordLeft => {
                pane.editor.extend(crate::state::EditorState::mv_word_left)
            }
            EditorKey::SelectWordRight => {
                pane.editor.extend(crate::state::EditorState::mv_word_right)
            }
            EditorKey::SelectHome => pane.editor.extend(crate::state::EditorState::mv_home),
            EditorKey::SelectEnd => pane.editor.extend(crate::state::EditorState::mv_end),
            EditorKey::Insert(s) if autocomplete_pairs => pane.editor.insert_with_pairs(&s),
            EditorKey::Insert(s) => pane.editor.insert(&s),
            EditorKey::Left => pane.editor.left(),
            EditorKey::Right => pane.editor.right(),
            EditorKey::Home => pane.editor.home(),
            EditorKey::End => pane.editor.end(),
            EditorKey::WordLeft => pane.editor.word_left(),
            EditorKey::WordRight => pane.editor.word_right(),
            EditorKey::Backspace if autocomplete_pairs => pane.editor.backspace_with_pairs(),
            EditorKey::Backspace => pane.editor.backspace(),
            EditorKey::Delete => pane.editor.delete(),
            EditorKey::DeleteWordBack => pane.editor.delete_word_back(),
            EditorKey::DeleteWordForward => pane.editor.delete_word_forward(),
            EditorKey::KillToEnd => pane.editor.kill_to_end(),
            EditorKey::KillToStart => pane.editor.kill_to_start(),
            EditorKey::Undo => pane.editor.undo(),
            EditorKey::Redo => pane.editor.redo(),
            EditorKey::ClearLine => pane.editor.clear(),
            EditorKey::ClearScreen => {
                // shell-driven repaint: blocks and scrollback survive (⌘K is
                // the full app-side clear)
                pane.session.write(b"\x0c");
                pane.view_offset = 0;
                edited = false;
            }
            EditorKey::HistoryUp => pane.editor.history_up(),
            EditorKey::HistoryDown => pane.editor.history_down(),
            EditorKey::Dismiss => {
                pane.editor.exit_placeholder_mode();
                edited = false;
            }
            EditorKey::Complete | EditorKey::HistorySearch | EditorKey::AcceptGhostWord => {
                edited = false
            }
            EditorKey::Submit => {
                pane.correction = None;
                let cmd = pane.editor.submit();
                pane.view_offset = 0;
                pane.selected_block = None;
                // stamp prompt-time context onto the block for its header
                let cwd = pane.session.cwd.lock().unwrap().clone();
                let branch = pane.git.as_ref().map(|g| g.branch.clone());
                pane.session
                    .blocks
                    .lock()
                    .unwrap()
                    .stamp_context(cwd, branch);
                pane.session.write(cmd.as_bytes());
                pane.session.write(b"\n");
                submitted = Some((pane.session.id, cmd));
                self.completion.close();
                edited = false;
            }
            EditorKey::Interrupt => {
                pane.editor.clear();
                pane.session.write(b"\x03");
                signal = Some((pane.session.id, b"\x03"));
                self.completion.close();
                edited = false;
            }
            EditorKey::Eof => {
                if pane.editor.buffer.is_empty() {
                    pane.session.write(b"\x04");
                    signal = Some((pane.session.id, b"\x04"));
                }
                edited = false;
            }
        }
        if let Some(text) = cut {
            self.copy_to_clipboard(text);
        }
        if let Some((id, bytes)) = signal {
            self.write_synced(id, bytes, true);
        }
        // synchronized inputs: the same command runs in every pane of the tab
        if let Some((id, cmd)) = submitted {
            for other in self.input_targets(id).into_iter().skip(1) {
                self.submit_to(other, &cmd);
            }
        }
        if edited {
            if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                pane.clear_selection();
                pane.selected_block = None; // typing returns focus to the input
                pane.correction = None;
            }
            self.update_completions();
        }
        self.blink_epoch = Instant::now(); // cursor solid while editing
    }

    fn find_tab_of_pane(&mut self, pane: PaneId) -> Option<(usize, &mut TabState)> {
        self.tabs
            .iter_mut()
            .enumerate()
            .find(|(_, t)| t.panes.iter().any(|p| p.session.id == pane))
    }

    /// Commands still running inside `scope` (the close warning lists them).
    fn running_in(&self, scope: CloseScope) -> Vec<String> {
        let panes: Vec<&PaneState> = match scope {
            CloseScope::Pane(id) => self.tabs.iter().filter_map(|t| t.pane(id)).collect(),
            CloseScope::Tab(i) => self
                .tabs
                .get(i)
                .map(|t| t.panes.iter().collect())
                .unwrap_or_default(),
            CloseScope::OtherTabs(keep) => self
                .tabs
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != keep)
                .flat_map(|(_, t)| t.panes.iter())
                .collect(),
            CloseScope::Window => self.tabs.iter().flat_map(|t| t.panes.iter()).collect(),
        };
        panes
            .into_iter()
            .filter(|p| p.session.has_foreground_job())
            .map(|p| {
                let running = p
                    .session
                    .blocks
                    .lock()
                    .unwrap()
                    .last()
                    .filter(|b| matches!(b.state, BlockState::Running { .. }))
                    .map(|b| b.command.clone())
                    .filter(|c| !c.is_empty());
                running
                    .or_else(|| p.session.foreground_job_name())
                    .unwrap_or_else(|| "process".into())
            })
            .collect()
    }

    /// Close/quit entry point: warns first when something is still running.
    fn request_close(&mut self, scope: CloseScope, event_loop: &ActiveEventLoop) {
        if self.config.confirm_close_running {
            let commands = self.running_in(scope);
            if !commands.is_empty() {
                self.confirm = Some(ConfirmClose { scope, commands });
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
        }
        self.perform_close(scope, event_loop);
    }

    fn perform_close(&mut self, scope: CloseScope, event_loop: &ActiveEventLoop) {
        match scope {
            CloseScope::Pane(id) => self.close_pane(id, event_loop),
            CloseScope::Tab(i) => {
                let ids: Vec<PaneId> = self
                    .tabs
                    .get(i)
                    .map(|t| t.panes.iter().map(|p| p.session.id).collect())
                    .unwrap_or_default();
                for id in ids {
                    self.close_pane(id, event_loop);
                }
            }
            CloseScope::OtherTabs(keep) => {
                let ids: Vec<PaneId> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != keep)
                    .flat_map(|(_, t)| t.panes.iter().map(|p| p.session.id))
                    .collect();
                for id in ids {
                    self.close_pane(id, event_loop);
                }
                self.active = 0;
                self.save_session();
            }
            CloseScope::Window => {
                self.save_session();
                event_loop.exit();
            }
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Confirm-modal keys: Esc cancels, Enter confirms; everything else is
    /// swallowed while the modal is up.
    fn handle_confirm_key(&mut self, event: &input::KeyPress, event_loop: &ActiveEventLoop) {
        use winit::keyboard::{Key, NamedKey};
        match event.logical_key {
            Key::Named(NamedKey::Escape) => self.confirm = None,
            Key::Named(NamedKey::Enter) => {
                if let Some(c) = self.confirm.take() {
                    self.perform_close(c.scope, event_loop);
                }
            }
            _ => {}
        }
    }

    /// Open another Runway window (a separate process that neither restores
    /// nor overwrites this window's saved session).
    fn open_new_window(&self) {
        let cwd = self.start_dir();
        match std::env::current_exe() {
            Ok(exe) => {
                let mut cmd = std::process::Command::new(exe);
                cmd.env("RUNWAY_SECONDARY_WINDOW", "1")
                    .stdin(std::process::Stdio::null());
                if let Some(dir) = cwd {
                    cmd.current_dir(dir);
                }
                if let Err(e) = cmd.spawn() {
                    log::warn!("new window: {e}");
                }
            }
            Err(e) => log::warn!("new window: {e}"),
        }
    }

    /// Directory for a new tab/pane per `working_directory`.
    fn start_dir(&self) -> Option<std::path::PathBuf> {
        let previous = self
            .tabs
            .get(self.active)
            .and_then(|t| t.active())
            .and_then(|p| p.session.cwd.lock().unwrap().clone());
        self.config.start_dir(previous.as_deref())
    }

    /// Panes that receive input typed into `pane` (itself, plus every other
    /// pane of its tab while synchronized inputs are on).
    fn input_targets(&self, pane: PaneId) -> Vec<PaneId> {
        match self.tabs.iter().find(|t| t.pane(pane).is_some()) {
            Some(t) if t.sync_inputs => {
                let mut ids = vec![pane];
                ids.extend(
                    t.panes
                        .iter()
                        .map(|p| p.session.id)
                        .filter(|id| *id != pane),
                );
                ids
            }
            _ => vec![pane],
        }
    }

    /// Byte offset in the active editor under the mouse (mono font: the
    /// column is found by measuring the row's shaped prefix widths).
    fn editor_offset_at_mouse(&mut self, pane_id: PaneId) -> Option<usize> {
        let (text_x, input_y) = self.editor_anchor;
        let (mx, my) = self.mouse;
        let buffer = self
            .tabs
            .get(self.active)?
            .pane(pane_id)?
            .editor
            .buffer
            .clone();
        let rows = editor::wrap_rows(&buffer, self.editor_cols);
        let row = (((my - input_y) / editor::LINE_H).floor().max(0.0) as usize).min(rows.len() - 1);
        let row_start = rows[row].start;
        let line = &buffer[rows[row].clone()];
        let engine = self.text.as_mut()?;
        let mut col = line.chars().count();
        let mut prev = 0.0f32;
        for (i, (bi, ch)) in line.char_indices().enumerate() {
            let w = engine.measure(
                &line[..bi + ch.len_utf8()],
                editor::FONT_SIZE,
                runway_renderer::mono_family(),
                runway_renderer::glyphon::Weight::NORMAL,
            );
            if mx - text_x < (prev + w) / 2.0 {
                col = i;
                break;
            }
            prev = w;
        }
        let within = line
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        Some(row_start + within)
    }

    /// Mouse press in a pane's editor: focus it, then place the cursor
    /// (double-click selects a word, triple-click the whole command, Shift
    /// extends the selection).
    fn click_editor(&mut self, pane_id: PaneId, clicks: u8) {
        if let Some(tab) = self.tabs.get_mut(self.active)
            && tab.active_pane != pane_id
        {
            if let Some(prev) = tab.active_mut() {
                prev.selected_block = None;
            }
            tab.active_pane = pane_id;
            return; // first click only focuses; geometry is the old pane's
        }
        let shift = self.modifiers.shift_key();
        let Some(offset) = self.editor_offset_at_mouse(pane_id) else {
            return;
        };
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        else {
            return;
        };
        pane.clear_selection();
        pane.selected_block = None;
        let e = &mut pane.editor;
        match clicks {
            2 => {
                let is_word = |c: char| !c.is_whitespace();
                let start = e.buffer[..offset]
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| is_word(*c))
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(offset);
                let end = e.buffer[offset..]
                    .char_indices()
                    .find(|(_, c)| !is_word(*c))
                    .map(|(i, _)| offset + i)
                    .unwrap_or(e.buffer.len());
                e.click_to(start, false);
                e.click_to(end, true);
            }
            3 => e.select_all(),
            _ => e.click_to(offset, shift),
        }
        self.completion.close();
        self.blink_epoch = Instant::now();
    }

    fn drag_editor(&mut self, pane_id: PaneId) {
        let Some(offset) = self.editor_offset_at_mouse(pane_id) else {
            return;
        };
        if let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(pane_id))
        {
            pane.editor.click_to(offset, true);
        }
    }

    /// Begin an inline rename: the current title is pre-filled and fully
    /// selected, so the first keystroke replaces it (Backspace clears it).
    fn start_rename(&mut self, i: usize) {
        if let Some(t) = self.tabs.get(i) {
            self.rename = Some((i, t.display_title()));
            self.rename_fresh = true;
        }
    }

    /// Inline tab rename keys: type to edit, Enter commits, Esc cancels.
    fn handle_rename_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let Some((i, buf)) = self.rename.as_mut() else {
            return;
        };
        let i = *i;
        let fresh = std::mem::take(&mut self.rename_fresh);
        if fresh
            && matches!(
                &event.logical_key,
                Key::Named(NamedKey::Backspace | NamedKey::Space) | Key::Character(_)
            )
        {
            buf.clear();
            if matches!(event.logical_key, Key::Named(NamedKey::Backspace)) {
                return;
            }
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.rename = None,
            Key::Named(NamedKey::Enter) => self.commit_rename(i),
            Key::Named(NamedKey::Backspace) => {
                if self.modifiers.super_key() || self.modifiers.alt_key() {
                    buf.clear();
                } else {
                    buf.pop();
                }
            }
            Key::Named(NamedKey::Space) => buf.push(' '),
            _ => {
                if !self.modifiers.super_key()
                    && !self.modifiers.control_key()
                    && let Some(text) = &event.text
                    && !text.chars().any(|c| c.is_control())
                    && buf.chars().count() < 60
                {
                    buf.push_str(text);
                }
            }
        }
        self.blink_epoch = Instant::now();
    }

    /// Apply the inline rename; an empty name restores the automatic title.
    fn commit_rename(&mut self, i: usize) {
        if let Some((_, name)) = self.rename.take()
            && let Some(tab) = self.tabs.get_mut(i)
        {
            let name = name.trim().to_string();
            tab.custom_title = (!name.is_empty()).then_some(name);
            self.save_session();
        }
    }

    /// Mirror raw bytes typed into `pane` to its synchronized siblings
    /// (`skip_self`: the caller already wrote to `pane`).
    fn write_synced(&mut self, pane: PaneId, bytes: &[u8], skip_self: bool) {
        let targets = self.input_targets(pane);
        for id in targets.into_iter().skip(usize::from(skip_self)) {
            if let Some((_, tab)) = self.find_tab_of_pane(id)
                && let Some(p) = tab.pane_mut(id)
            {
                if skip_self && bytes == b"\x03" {
                    p.editor.clear();
                }
                p.view_offset = 0;
                p.session.write(bytes);
            }
        }
    }

    /// Run `cmd` in pane `id` at its prompt: history, prompt-time context,
    /// then the bytes (used by submit, re-run, palette and synced inputs).
    fn submit_to(&mut self, id: PaneId, cmd: &str) {
        let Some((_, tab)) = self.find_tab_of_pane(id) else {
            return;
        };
        let Some(pane) = tab.pane_mut(id) else { return };
        if !cmd.trim().is_empty() && pane.editor.history.last().map(String::as_str) != Some(cmd) {
            pane.editor.history.push(cmd.to_string());
        }
        pane.view_offset = 0;
        pane.selected_block = None;
        let cwd = pane.session.cwd.lock().unwrap().clone();
        let branch = pane.git.as_ref().map(|g| g.branch.clone());
        pane.session
            .blocks
            .lock()
            .unwrap()
            .stamp_context(cwd, branch);
        pane.session.write(cmd.as_bytes());
        pane.session.write(b"\n");
    }
}

impl App {
    /// One key press (real or scripted). Routes overlays, app actions,
    /// the owned editor and raw PTY passthrough.
    pub(crate) fn on_key(
        &mut self,
        event: &input::KeyPress,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        if self.confirm.is_some() {
            self.handle_confirm_key(event, event_loop);
            window.request_redraw();
            return;
        }
        if self.rename.is_some() {
            self.handle_rename_key(event);
            window.request_redraw();
            return;
        }
        if self.context_menu.open {
            if !self.handle_menu_key(event, event_loop) {
                self.context_menu.close();
            }
            window.request_redraw();
            return;
        }
        if self.palette.open {
            if input::app_action(event, self.modifiers, &self.keymap)
                == Some(AppAction::TogglePalette)
            {
                self.palette.open = false;
            } else {
                self.handle_palette_key(event, event_loop);
            }
            window.request_redraw();
            return;
        }
        if self.settings.open {
            if self.handle_settings_key(event, window) {
                window.request_redraw();
            }
            return;
        }
        if self.search.open && self.filter_editing {
            self.handle_filter_key(event);
            window.request_redraw();
            return;
        }
        if self.search.open {
            let action = input::app_action(event, self.modifiers, &self.keymap);
            if action == Some(AppAction::Search) {
                self.search.close();
            } else if action == Some(AppAction::Copy) {
                self.handle_action(AppAction::Copy, event_loop);
            } else {
                self.handle_search_key(event, self.modifiers);
            }
            window.request_redraw();
            return;
        }
        // Escape clears the keyboard block selection before anything
        // else; in raw mode it still falls through so vim gets it.
        if matches!(
            event.logical_key,
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
        ) && let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut())
            && pane.selected_block.is_some()
        {
            pane.selected_block = None;
            window.request_redraw();
            if self.editor_active {
                return;
            }
        }
        if let Some(action) = input::app_action(event, self.modifiers, &self.keymap) {
            // plain terminal keys (PageUp, Home, ⌃M…) bound to app
            // actions still belong to vim/less when the editor isn't
            // active — fall through to raw byte passthrough
            if !action.needs_editor() || self.editor_active {
                log::debug!("key action: {action:?} (mods {:?})", self.modifiers);
                self.handle_action(action, event_loop);
                return;
            }
        }
        log::debug!(
            "key fallthrough: {:?} mods={:?} editor={}",
            event.logical_key,
            self.modifiers,
            self.editor_active
        );
        if self.editor_active {
            if self.vim_intercept(event) {
                window.request_redraw();
                return;
            }
            if let Some(key) = input::editor_key(event, self.modifiers) {
                self.handle_editor_key(key);
                window.request_redraw();
            }
            return;
        }
        let Some(id) = self.tabs.get(self.active).map(|t| t.active_pane) else {
            return;
        };
        let app_cursor = self
            .tabs
            .get(self.active)
            .and_then(|t| t.pane(id))
            .is_some_and(|p| {
                use runway_pty::alacritty_terminal::term::TermMode;
                p.session.term.lock().mode().contains(TermMode::APP_CURSOR)
            });
        let enc = input::KeyEncoding {
            app_cursor,
            option_as_meta: self.config.option_as_meta,
        };
        if let Some(bytes) = input::key_to_bytes(event, self.modifiers, enc) {
            if let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) {
                // typing into a program scrolls back to it and drops the selection
                pane.view_offset = 0;
                pane.clear_selection();
            }
            self.write_synced(id, &bytes, false);
        }
    }

    pub(crate) fn on_cursor_moved(&mut self, pos: (f32, f32), window: &Window) {
        self.mouse = pos;
        self.update_cursor_icon(window);
        if self.mouse_report.is_some() {
            self.report_motion();
            return;
        }
        let idle = !self.palette.open
            && !self.settings.open
            && !self.context_menu.open
            && self.confirm.is_none()
            && self.drag.is_none()
            && self.pending_sel.is_none()
            && self.editor_drag.is_none()
            && self.tab_drag.is_none()
            && self.slider_drag.is_none();
        if idle && self.report_motion() {
            return;
        }
        if let Some(drag) = &self.drag {
            let [ax, ay, aw, ah] = drag.area;
            let ratio = match drag.axis {
                Axis::Horizontal => (self.mouse.0 - ax) / (aw - crate::pane::HANDLE),
                Axis::Vertical => (self.mouse.1 - ay) / (ah - crate::pane::HANDLE),
            };
            let path = drag.path.clone();
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.root.set_ratio(&path, ratio);
            }
            window.request_redraw();
        }
        if self.context_menu.open {
            let (mx2, my2) = self.mouse;
            let hover = self
                .context_menu_rects
                .iter()
                .position(|r| mx2 >= r[0] && mx2 < r[0] + r[2] && my2 >= r[1] && my2 < r[1] + r[3]);
            if hover != self.context_menu.selected {
                self.context_menu.selected = hover;
                window.request_redraw();
            }
        }
        if self.pending_sel.is_some() {
            self.drag_selection();
        }
        if let Some(pane) = self.editor_drag {
            self.drag_editor(pane);
            window.request_redraw();
        }
        if let Some((i, x0)) = self.tab_drag {
            // drag a tab past a neighbour's midpoint to swap (Warp reorder)
            let dx = self.mouse.0 - x0;
            let neighbour = self.hits.iter().find_map(|h| match h.target {
                ClickTarget::TabSelect(j)
                    if (dx > 0.0 && j == i + 1) || (dx < 0.0 && j + 1 == i) =>
                {
                    Some((j, h.rect))
                }
                _ => None,
            });
            if let Some((j, r)) = neighbour {
                let mid = r[0] + r[2] / 2.0;
                if (dx > 0.0 && self.mouse.0 > mid) || (dx < 0.0 && self.mouse.0 < mid) {
                    self.tabs.swap(i, j);
                    self.active = j;
                    self.tab_drag = Some((j, self.mouse.0));
                    self.save_session();
                    window.request_redraw();
                }
            }
        }
        if let Some((id, rect)) = self.slider_drag {
            let t = ((self.mouse.0 - rect[0] - 8.0) / (rect[2] - 16.0)).clamp(0.0, 1.0);
            self.set_slider(id, t);
            window.request_redraw();
        }
    }

    pub(crate) fn on_right_press(&mut self, window: &Window) {
        if self.palette.open || self.settings.open || self.confirm.is_some() {
            return;
        }
        if !self.context_menu.open && self.report_button(crate::mouse::RIGHT, true) {
            window.request_redraw();
            return;
        }
        let (mx, my) = self.mouse;
        // tab bar: tab menu
        let tab_hit = self
            .hits
            .iter()
            .find(|h| h.contains(mx, my))
            .and_then(|h| match h.target {
                ClickTarget::TabSelect(i) | ClickTarget::TabClose(i) => Some(i),
                _ => None,
            });
        if let Some(i) = tab_hit {
            self.open_tab_menu(i, self.mouse);
            window.request_redraw();
            return;
        }
        // resolve the block under the cursor via the frame row cache
        let target = self.tabs.get(self.active).and_then(|tab| {
            tab.panes.iter().find_map(|p| {
                let grid = p.last_grid?;
                if my < grid.y || p.rows.is_empty() {
                    return None;
                }
                let idx = (((my - grid.y) / grid.line_h).floor() as i64)
                    .clamp(0, p.rows.len() as i64 - 1) as usize;
                if mx < grid.x || mx > grid.right {
                    return None;
                }
                use crate::views::terminal::RowKind;
                let blocks = p.session.blocks.lock().unwrap();
                match p.rows[idx].kind {
                    RowKind::Pad => None,
                    RowKind::Header(id) => Some((p.session.id, id)),
                    RowKind::Grid { abs, .. } => blocks
                        .iter()
                        .find(|b| {
                            b.suppressed_range().is_some()
                                && abs >= b.prompt_start
                                && abs < b.end.unwrap_or(u64::MAX)
                        })
                        .map(|b| (p.session.id, b.id)),
                }
            })
        });
        if let Some((pane_id, block)) = target {
            self.open_context_menu(pane_id, block, self.mouse);
            window.request_redraw();
            return;
        }
        // anywhere else in a pane: terminal menu for the pane under the mouse
        let (lw, lh) = self.logical_size();
        let (rects, _) = self.pane_rects(lw, lh);
        if let Some(pr) = rects.iter().find(|r| {
            mx >= r.rect[0]
                && mx < r.rect[0] + r.rect[2]
                && my >= r.rect[1]
                && my < r.rect[1] + r.rect[3]
        }) {
            let id = pr.id;
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.active_pane = id;
            }
            self.open_terminal_menu(id, self.mouse);
            window.request_redraw();
        }
    }

    pub(crate) fn on_left(
        &mut self,
        state: ElementState,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        if state == ElementState::Released {
            if self.report_button(crate::mouse::LEFT, false) {
                window.request_redraw();
                return;
            }
            if self.drag.take().is_some() {
                self.save_session(); // divider position
            }
            self.tab_drag = None;
            if self.editor_drag.take().is_some() {
                return;
            }
            let selecting = self.pending_sel.take().is_some_and(|p| p.started);
            if selecting
                && self.config.copy_on_select
                && let Some(text) = self.selection_text()
            {
                self.copy_to_clipboard(text);
            }
            if self.slider_drag.take().is_some() {
                self.save_config();
            }
            return;
        }
        let (mx, my) = self.mouse;
        let inside =
            |r: &[f32; 4]| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3];
        if self.confirm.is_some() {
            if inside(&self.confirm_rects[1]) {
                if let Some(c) = self.confirm.take() {
                    self.perform_close(c.scope, event_loop);
                }
            } else if inside(&self.confirm_rects[0]) {
                self.confirm = None;
            }
            window.request_redraw();
            return;
        }
        if let Some((i, _)) = &self.rename {
            // clicking anywhere commits the rename
            let i = *i;
            self.commit_rename(i);
        }
        if self.context_menu.open {
            let row = self
                .context_menu_rects
                .iter()
                .position(|r| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]);
            match row {
                Some(i) => {
                    let item = &self.context_menu.items[i];
                    if item.enabled {
                        let action = item.action;
                        self.run_menu_action(action, event_loop);
                    }
                }
                None => self.context_menu.close(),
            }
            window.request_redraw();
            return;
        }
        if self.palette.open {
            let row = self
                .palette_row_hits
                .iter()
                .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
                .map(|(_, i)| *i);
            match row {
                Some(i) => {
                    if let Some(m) = self.palette_results.get(i) {
                        let action = m.item.action.clone();
                        self.run_palette_action(action, event_loop);
                    }
                }
                None => self.palette.open = false,
            }
            window.request_redraw();
            return;
        }
        // find-bar toggles
        if self.search.open
            && !self.filter_editing
            && let Some(i) = self.search_chips.iter().position(inside)
        {
            if i == 0 {
                self.search.case_sensitive = !self.search.case_sensitive;
            } else {
                self.search.regex = !self.search.regex;
            }
            self.refresh_search(true);
            window.request_redraw();
            return;
        }
        // a program tracking the mouse (vim, tmux, htop) gets the click
        if !self.settings.open && self.report_button(crate::mouse::LEFT, true) {
            self.last_click = None;
            window.request_redraw();
            return;
        }
        let target = self
            .hits
            .iter()
            .find(|h| h.contains(mx, my))
            .map(|h| h.target.clone());
        // any click but another row ends a shortcut recording
        if !matches!(target, Some(ClickTarget::SettingsKeybind(_))) {
            self.settings.recording = None;
        }
        match &target {
            Some(ClickTarget::Block(pane, id)) => {
                // Warp: clicking a block selects it (again deselects)
                let (pane, id) = (*pane, *id);
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if tab.active_pane != pane
                        && let Some(prev) = tab.active_mut()
                    {
                        prev.selected_block = None;
                    }
                    tab.active_pane = pane;
                    if let Some(p) = tab.pane_mut(pane) {
                        p.selected_block = if p.selected_block == Some(id) {
                            None
                        } else {
                            Some(id)
                        };
                    }
                }
            }
            Some(ClickTarget::BlockChevron(pane, id)) => {
                if let Some((_, tab)) = self.find_tab_of_pane(*pane)
                    && let Some(p) = tab.pane(*pane)
                {
                    p.session.blocks.lock().unwrap().toggle_collapsed(*id);
                }
            }
            Some(ClickTarget::TabSelect(i)) => {
                let i = *i;
                if i < self.tabs.len() {
                    // double-click renames (Warp); a press arms drag-reorder
                    if self.click_count() == 2 {
                        self.start_rename(i);
                    }
                    if i != self.active {
                        self.switch_tab(i);
                    }
                    self.tab_drag = Some((i, mx));
                }
            }
            Some(ClickTarget::TabClose(i)) => {
                // closing a tab closes all its panes (warns if busy)
                self.request_close(CloseScope::Tab(*i), event_loop);
            }
            Some(ClickTarget::TabNew) => self.spawn_tab(),
            Some(ClickTarget::Editor(pane)) => {
                let pane = *pane;
                let clicks = self.click_count();
                self.click_editor(pane, clicks);
                self.editor_drag = Some(pane);
            }
            Some(ClickTarget::PaneFocus(id)) => {
                let id = *id;
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if tab.active_pane != id
                        && let Some(prev) = tab.active_mut()
                    {
                        prev.selected_block = None;
                    }
                    tab.active_pane = id;
                }
                let clicks = self.click_count();
                if self.try_open_url(id) {
                    self.last_click = None;
                } else {
                    self.press_selection(id, clicks);
                }
            }
            Some(ClickTarget::HandleDrag(path)) => {
                // find the split node's area from the current layout
                let (lw, lh) = self.logical_size();
                let (_, handles) = self.pane_rects(lw, lh);
                if let Some(hr) = handles.iter().find(|h| &h.path == path) {
                    self.drag = Some(DragState {
                        path: path.clone(),
                        axis: hr.axis,
                        area: hr.area,
                    });
                }
            }
            Some(ClickTarget::BlockCopy(pane, id)) => {
                let text = self
                    .find_tab_of_pane(*pane)
                    .and_then(|(_, tab)| tab.pane(*pane))
                    .and_then(|p| terminal::block_output_text(&p.session, *id));
                if let Some(text) = text {
                    self.copy_to_clipboard(text);
                }
            }
            Some(ClickTarget::TrafficLight(i)) => match i {
                0 => self.request_close(CloseScope::Window, event_loop),
                1 => window.set_minimized(true),
                _ => window.set_maximized(!window.is_maximized()),
            },
            Some(ClickTarget::WindowDrag) => {
                let _ = window.drag_window();
            }
            Some(ClickTarget::JumpToBlock(pane, id)) => {
                self.jump_to_block(*pane, *id);
            }
            Some(ClickTarget::SettingsNoop) => {}
            Some(ClickTarget::SettingsNav(i)) => {
                use crate::views::settings::Page;
                let page = Page::ALL.get(*i as usize).copied().unwrap_or(Page::General);
                self.settings.open_page(page);
            }
            Some(ClickTarget::SettingsClose) => {
                self.settings.open = false;
            }
            Some(ClickTarget::SettingsToggle(id)) => {
                let c = &mut self.config;
                match *id {
                    "cursor_blink" => c.cursor_blink = !c.cursor_blink,
                    "ligatures" => c.ligatures = !c.ligatures,
                    "sync_os_theme" => c.sync_os_theme = !c.sync_os_theme,
                    "restore_session" => c.restore_session = !c.restore_session,
                    "vim_mode" => c.vim_mode = !c.vim_mode,
                    "autosuggestions" => c.autosuggestions = !c.autosuggestions,
                    "completions_menu" => c.completions_menu = !c.completions_menu,
                    "autocomplete_pairs" => c.autocomplete_pairs = !c.autocomplete_pairs,
                    "copy_on_select" => c.copy_on_select = !c.copy_on_select,
                    "notify_on_completion" => c.notify_on_completion = !c.notify_on_completion,
                    "alias_expansion" => c.alias_expansion = !c.alias_expansion,
                    "error_underline" => c.error_underline = !c.error_underline,
                    "command_corrections" => c.command_corrections = !c.command_corrections,
                    "confirm_close_running" => c.confirm_close_running = !c.confirm_close_running,
                    "mouse_reporting" => c.mouse_reporting = !c.mouse_reporting,
                    "option_as_meta" => c.option_as_meta = !c.option_as_meta,
                    _ => {}
                }
                self.save_config();
                self.apply_config();
            }
            Some(ClickTarget::SettingsStep(id, dir)) => {
                let c = &mut self.config;
                let d = *dir as i64;
                match *id {
                    "font_size" => {
                        c.font_size = (c.font_size + d as f32).clamp(8.0, 28.0);
                    }
                    "scrollback" => {
                        c.scrollback =
                            (c.scrollback as i64 + d * 1000).clamp(1000, 1_000_000) as usize;
                    }
                    "notify_threshold_secs" => {
                        c.notify_threshold_secs =
                            (c.notify_threshold_secs as i64 + d * 5).clamp(0, 3600) as u64;
                    }
                    "window_padding" => {
                        c.window_padding = (c.padding() + d as f32 * 2.0).clamp(0.0, 40.0);
                    }
                    _ => {}
                }
                self.save_config();
                self.apply_config();
            }
            Some(ClickTarget::SettingsSegment(id, v)) => {
                use runway_core::{Bell, CursorStyle, InputPosition, PromptMode};
                let c = &mut self.config;
                match *id {
                    "cursor_style" => {
                        c.cursor_style = match v {
                            0 => CursorStyle::Block,
                            1 => CursorStyle::Bar,
                            _ => CursorStyle::Underline,
                        }
                    }
                    "input_position" => {
                        c.input_position = if *v == 1 {
                            InputPosition::Top
                        } else {
                            InputPosition::Bottom
                        }
                    }
                    "prompt" => {
                        c.prompt = if *v == 1 {
                            PromptMode::Shell
                        } else {
                            PromptMode::Runway
                        }
                    }
                    "working_directory" => match v {
                        0 => c.working_directory = "previous".into(),
                        1 => c.working_directory = "home".into(),
                        // a custom path is typed in config.toml; keep one already set
                        _ => {
                            if matches!(c.working_directory.trim(), "" | "previous" | "home") {
                                self.toast = Some((
                                    "Set working_directory = \"/path\" in config.toml".into(),
                                    Instant::now(),
                                ));
                            }
                        }
                    },
                    "bell" => {
                        c.bell = match v {
                            0 => Bell::Visual,
                            1 => Bell::Audible,
                            2 => Bell::Both,
                            _ => Bell::None,
                        }
                    }
                    _ => {}
                }
                self.save_config();
                self.apply_config();
            }
            Some(ClickTarget::SettingsCycle(id)) => {
                let names = match *id {
                    "theme" | "theme_light" => runway_core::theme::available_themes(),
                    "font_family" => self
                        .text
                        .as_ref()
                        .map(|t| t.monospace_families())
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                if !names.is_empty() {
                    let slot = match *id {
                        "theme" => &mut self.config.theme,
                        "theme_light" => &mut self.config.theme_light,
                        _ => &mut self.config.font_family,
                    };
                    let cur = names
                        .iter()
                        .position(|n| n.eq_ignore_ascii_case(slot))
                        .unwrap_or(names.len() - 1);
                    *slot = names[(cur + 1) % names.len()].clone();
                }
                self.save_config();
                self.apply_config();
            }
            Some(ClickTarget::SettingsSlider(id)) => {
                let rect = self
                    .hits
                    .iter()
                    .find(|h| matches!(&h.target, ClickTarget::SettingsSlider(s) if s == id))
                    .map(|h| h.rect);
                if let Some(rect) = rect {
                    self.slider_drag = Some((id, rect));
                    let t = ((mx - rect[0] - 8.0) / (rect[2] - 16.0)).clamp(0.0, 1.0);
                    self.set_slider(id, t);
                }
            }
            Some(ClickTarget::SettingsKeybind(action)) => {
                self.settings.recording = Some(action.clone());
            }
            Some(ClickTarget::SettingsKeybindReset(action)) => {
                self.settings.recording = None;
                self.config.keybindings.remove(action);
                self.save_config();
                self.apply_config();
                self.toast(format!("{action} restored to its default shortcut"));
            }
            Some(ClickTarget::SettingsTheme(i)) => {
                let names = runway_core::theme::available_themes();
                if let Some(name) = names.into_iter().nth(*i as usize) {
                    self.config.theme = name;
                    self.save_config();
                    self.apply_config();
                }
            }
            None => {
                let active_pane = self.tabs.get(self.active).map(|t| t.active_pane);
                if let Some(id) = active_pane {
                    let clicks = self.click_count();
                    if self.try_open_url(id) {
                        self.last_click = None;
                    } else {
                        self.press_selection(id, clicks);
                    }
                    window.request_redraw();
                }
            }
        }
        if target.is_some() {
            window.request_redraw();
        }
    }

    pub(crate) fn on_wheel(&mut self, delta: MouseScrollDelta, window: &Window) {
        use runway_pty::alacritty_terminal::term::TermMode;
        // accumulate fractional deltas: trackpads emit many sub-line
        // pixel events that would each truncate to zero lines
        self.scroll_accum += match delta {
            MouseScrollDelta::LineDelta(_, y) => y * 3.0,
            MouseScrollDelta::PixelDelta(p) => p.y as f32 / self.line_h,
        };
        let lines = self.scroll_accum as i32;
        self.scroll_accum -= lines as f32;
        if lines == 0 {
            return;
        }
        if self.settings.open {
            let (_, lh) = self.logical_size();
            self.settings
                .scroll_by(lines as f32, lh - TAB_BAR_H - 100.0);
            window.request_redraw();
            return;
        }
        if self.palette.open || self.confirm.is_some() || self.context_menu.open {
            return;
        }
        // the wheel scrolls the pane under the pointer (Warp), not the focused one
        let Some(target) = self
            .pane_under_mouse()
            .or_else(|| self.tabs.get(self.active).map(|t| t.active_pane))
        else {
            return;
        };
        // mouse-tracking app: wheel reports at the pointer
        if let Some(mode) = self.mouse_tracking(target)
            && let Some(cell) = self.mouse_cell(target)
        {
            let button = if lines > 0 {
                crate::mouse::WHEEL_UP
            } else {
                crate::mouse::WHEEL_DOWN
            };
            for _ in 0..lines.unsigned_abs().min(20) {
                self.send_mouse(target, mode, button, true, false, cell);
            }
            return;
        }
        let Some(pane) = self
            .tabs
            .get_mut(self.active)
            .and_then(|t| t.pane_mut(target))
        else {
            return;
        };
        let mode = *pane.session.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN) {
            // alternate scroll (DECSET 1007): the wheel becomes arrow keys
            // for full-screen apps without mouse tracking (less, man)
            if mode.contains(TermMode::ALTERNATE_SCROLL) {
                let app = mode.contains(TermMode::APP_CURSOR);
                let seq: &[u8] = match (lines > 0, app) {
                    (true, false) => b"\x1b[A",
                    (true, true) => b"\x1bOA",
                    (false, false) => b"\x1b[B",
                    (false, true) => b"\x1bOB",
                };
                for _ in 0..lines.unsigned_abs().min(20) {
                    pane.session.write(seq);
                }
            }
        } else {
            pane.view_offset = if lines > 0 {
                pane.view_offset.saturating_add(lines as usize)
            } else {
                pane.view_offset
                    .saturating_sub(lines.unsigned_abs() as usize)
            };
            window.request_redraw();
        }
    }

    /// Pane whose rect (in the active tab) contains the pointer.
    fn pane_under_mouse(&self) -> Option<PaneId> {
        let (lw, lh) = self.logical_size();
        let (rects, _) = self.pane_rects(lw, lh);
        let (mx, my) = self.mouse;
        rects
            .iter()
            .find(|r| {
                mx >= r.rect[0]
                    && mx < r.rect[0] + r.rect[2]
                    && my >= r.rect[1]
                    && my < r.rect[1] + r.rect[3]
            })
            .map(|r| r.id)
    }

    /// Terminal mode of `pane` when its program tracks the mouse and
    /// reporting applies (Shift forces local selection, xterm-style).
    fn mouse_tracking(
        &self,
        pane: PaneId,
    ) -> Option<runway_pty::alacritty_terminal::term::TermMode> {
        use runway_pty::alacritty_terminal::term::TermMode;
        if !self.config.mouse_reporting || self.modifiers.shift_key() {
            return None;
        }
        let p = self.tabs.get(self.active)?.pane(pane)?;
        let mode = *p.session.term.lock().mode();
        mode.intersects(TermMode::MOUSE_MODE).then_some(mode)
    }

    /// 0-based screen cell under the pointer inside `pane`'s grid (None over
    /// history rows, block headers' gutters, or outside the grid).
    fn mouse_cell(&self, pane: PaneId) -> Option<(usize, usize)> {
        let p = self.tabs.get(self.active)?.pane(pane)?;
        let g = p.last_grid?;
        let (mx, my) = self.mouse;
        if mx < g.x || mx > g.right || my < g.y || my >= g.y + g.rows as f32 * g.line_h {
            return None;
        }
        let (point, _) = Self::grid_point_at(p, mx, my)?;
        (point.line.0 >= 0).then_some((point.column.0, point.line.0 as usize))
    }

    /// Write one mouse report to `pane`.
    fn send_mouse(
        &self,
        pane: PaneId,
        mode: runway_pty::alacritty_terminal::term::TermMode,
        button: u8,
        pressed: bool,
        motion: bool,
        cell: (usize, usize),
    ) {
        use runway_pty::alacritty_terminal::term::TermMode;
        let enc = crate::mouse::Encoding {
            sgr: mode.contains(TermMode::SGR_MOUSE),
            utf8: mode.contains(TermMode::UTF8_MOUSE),
        };
        if let Some(bytes) =
            crate::mouse::report(button, pressed, motion, self.modifiers, cell.0, cell.1, enc)
            && let Some(p) = self.tabs.get(self.active).and_then(|t| t.pane(pane))
        {
            p.session.write(&bytes);
        }
    }

    /// Button press/release over a mouse-tracking program. Returns true when
    /// the event was reported (it then drives no local selection or menu).
    fn report_button(&mut self, button: u8, pressed: bool) -> bool {
        use runway_pty::alacritty_terminal::term::TermMode;
        if !pressed {
            let Some((pane, held, cell)) = self.mouse_report else {
                return false;
            };
            if held != button {
                return false;
            }
            self.mouse_report = None;
            let mode = self
                .tabs
                .get(self.active)
                .and_then(|t| t.pane(pane))
                .map(|p| *p.session.term.lock().mode())
                .unwrap_or(TermMode::empty());
            if mode.intersects(TermMode::MOUSE_MODE) {
                let cell = self.mouse_cell(pane).unwrap_or(cell);
                self.send_mouse(pane, mode, button, false, false, cell);
            }
            return true;
        }
        let Some(pane) = self.pane_under_mouse() else {
            return false;
        };
        let Some(mode) = self.mouse_tracking(pane) else {
            return false;
        };
        let Some(cell) = self.mouse_cell(pane) else {
            return false;
        };
        // the click still focuses the pane it lands in
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.active_pane != pane
                && let Some(prev) = tab.active_mut()
            {
                prev.selected_block = None;
            }
            tab.active_pane = pane;
            if let Some(p) = tab.pane_mut(pane) {
                p.clear_selection();
            }
        }
        self.send_mouse(pane, mode, button, true, false, cell);
        self.mouse_report = Some((pane, button, cell));
        true
    }

    /// Pointer motion over a mouse-tracking program: drag reports (modes
    /// 1002/1003) while a button is held, hover reports (1003) otherwise.
    /// Returns true when the motion belongs to the program.
    fn report_motion(&mut self) -> bool {
        use runway_pty::alacritty_terminal::term::TermMode;
        if let Some((pane, button, last)) = self.mouse_report {
            let mode = self
                .tabs
                .get(self.active)
                .and_then(|t| t.pane(pane))
                .map(|p| *p.session.term.lock().mode())
                .unwrap_or(TermMode::empty());
            if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
                && let Some(cell) = self.mouse_cell(pane)
                && cell != last
            {
                self.send_mouse(pane, mode, button, true, true, cell);
                self.mouse_report = Some((pane, button, cell));
            }
            return true;
        }
        let Some(pane) = self.pane_under_mouse() else {
            return false;
        };
        let Some(mode) = self.mouse_tracking(pane) else {
            return false;
        };
        if !mode.contains(TermMode::MOUSE_MOTION) {
            return false;
        }
        let Some(cell) = self.mouse_cell(pane) else {
            return false;
        };
        if self.mouse_hover_cell != Some((pane, cell)) {
            self.mouse_hover_cell = Some((pane, cell));
            self.send_mouse(pane, mode, crate::mouse::NONE, true, true, cell);
        }
        true
    }

    /// Right/middle button release (press routing lives in on_right_press).
    pub(crate) fn on_other_button(&mut self, button: u8, pressed: bool, window: &Window) {
        let overlay = self.palette.open
            || self.settings.open
            || self.confirm.is_some()
            || self.context_menu.open;
        if pressed && overlay {
            return;
        }
        if self.report_button(button, pressed) {
            window.request_redraw();
        }
    }

    /// Text committed by the input method (or any composed text): routed
    /// exactly like typed characters.
    pub(crate) fn commit_text(
        &mut self,
        text: &str,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        self.preedit.clear();
        if text.is_empty() {
            return;
        }
        let press = input::KeyPress {
            logical_key: winit::keyboard::Key::Character(text.into()),
            text: Some(text.to_string()),
            unmodified: None,
        };
        let saved = self.modifiers;
        self.modifiers = ModifiersState::empty();
        self.on_key(&press, event_loop, window);
        self.modifiers = saved;
        window.request_redraw();
    }

    /// A menu bar item: replay its chord through `on_key` (identical to the
    /// shortcut); an action with no binding runs directly.
    #[cfg(target_os = "macos")]
    fn run_menu_command(&mut self, id: &str, event_loop: &ActiveEventLoop) {
        use crate::menubar::{MenuCommand, chord_for};
        let Some(cmd) = MenuCommand::from_id(id) else {
            return;
        };
        let Some(window) = self.window.clone() else {
            return;
        };
        let chord = match &cmd {
            MenuCommand::Action(a) => chord_for(&self.keymap, a),
            MenuCommand::Chord(c) => Some(c.clone()),
        };
        if let Some((press, mods)) = chord.as_deref().and_then(crate::script::key_press) {
            let saved = self.modifiers;
            self.modifiers = mods;
            self.on_key(&press, event_loop, &window);
            self.modifiers = saved;
        } else if let MenuCommand::Action(a) = cmd
            && let Some(action) = input::action_from_name(&a)
        {
            self.handle_action(action, event_loop);
        }
        window.request_redraw();
    }

    /// Tell the program in the active pane that the window gained/lost focus
    /// when it asked for focus events (DECSET 1004: vim autoread, tmux).
    fn report_focus(&self, focused: bool) {
        use runway_pty::alacritty_terminal::term::TermMode;
        if !self.config.mouse_reporting {
            return;
        }
        if let Some(p) = self.tabs.get(self.active).and_then(|t| t.active())
            && p.session
                .term
                .lock()
                .mode()
                .contains(TermMode::FOCUS_IN_OUT)
        {
            p.session.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
    }
}

impl App {
    /// Open every tab/split of a launch configuration and queue its commands.
    fn apply_launch_config(&mut self, cfg: &runway_core::launch::LaunchConfig) {
        let first_new = self.tabs.len();
        for tab in &cfg.tabs {
            let (cwd, _) = Self::first_leaf(&tab.layout);
            self.spawn_tab_at(cwd.filter(|p| p.is_dir()));
            let root_pane = self.tabs[self.active].active_pane;
            if let Some(title) = &tab.title {
                self.tabs[self.active].custom_title = Some(title.clone());
            }
            self.build_layout(&tab.layout, root_pane);
        }
        self.active = (first_new + cfg.active_tab).min(self.tabs.len().saturating_sub(1));
        self.save_session();
    }

    /// cwd + commands of the first (top-left) pane of a layout.
    fn first_leaf(
        layout: &runway_core::launch::LaunchLayout,
    ) -> (Option<std::path::PathBuf>, Vec<String>) {
        use runway_core::launch::LaunchLayout;
        match layout {
            LaunchLayout::Pane { cwd, commands, .. } => (cwd.clone(), commands.clone()),
            LaunchLayout::Split { panes, .. } => panes
                .first()
                .map(Self::first_leaf)
                .unwrap_or((None, Vec::new())),
        }
    }

    /// Realize `layout` inside the region currently held by `pane`.
    fn build_layout(&mut self, layout: &runway_core::launch::LaunchLayout, pane: PaneId) {
        use runway_core::launch::LaunchLayout;
        match layout {
            LaunchLayout::Pane {
                commands, focused, ..
            } => {
                if !commands.is_empty() {
                    self.pending_commands.insert(pane, commands.clone());
                }
                if *focused && let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = pane;
                }
            }
            LaunchLayout::Split { vertical, panes } => {
                let axis = if *vertical {
                    Axis::Horizontal // side by side
                } else {
                    Axis::Vertical
                };
                // pane holds panes[0]; each further child splits off the previous one
                let mut ids = vec![pane];
                let mut prev = pane;
                for child in panes.iter().skip(1) {
                    let (cwd, _) = Self::first_leaf(child);
                    let new_id = self.split_pane(prev, axis, cwd.filter(|p| p.is_dir()));
                    ids.push(new_id);
                    prev = new_id;
                }
                for (child, id) in panes.iter().zip(ids) {
                    self.build_layout(child, id);
                }
            }
        }
    }

    /// Split `target` along `axis`, spawning the new pane at `cwd`.
    fn split_pane(
        &mut self,
        target: PaneId,
        axis: Axis,
        cwd: Option<std::path::PathBuf>,
    ) -> PaneId {
        let session = self.spawn_session(60, 20, cwd); // resized on next layout
        let new_id = session.id;
        let tab = &mut self.tabs[self.active];
        tab.zoomed = None;
        if tab.root.split(target, new_id, axis) {
            tab.panes.push(PaneState::new(session));
            tab.active_pane = new_id;
        }
        new_id
    }

    /// Send a launch configuration's queued commands once the pane's shell
    /// shows its first prompt.
    fn flush_pending_commands(&mut self, pane: PaneId) {
        if !self.pending_commands.contains_key(&pane) {
            return;
        }
        let ready = self
            .find_tab_of_pane(pane)
            .and_then(|(_, tab)| tab.pane(pane))
            .is_some_and(|p| {
                p.session
                    .saw_output
                    .load(std::sync::atomic::Ordering::Acquire)
                    && matches!(
                        p.session.blocks.lock().unwrap().last().map(|b| &b.state),
                        Some(BlockState::Prompt)
                    )
            });
        if !ready {
            return;
        }
        let Some(cmds) = self.pending_commands.remove(&pane) else {
            return;
        };
        for cmd in cmds {
            self.submit_to(pane, &cmd);
        }
    }

    /// Settings slider value (0..1) → config field.
    fn set_slider(&mut self, id: &str, t: f32) {
        match id {
            "line_height" => self.config.line_height = 1.0 + 1.2 * t,
            "window_opacity" => self.config.window_opacity = 0.3 + 0.7 * t,
            _ => return,
        }
        self.apply_config();
    }

    /// Vim modal editing: route the key through the editor's vim state
    /// first. Returns true when vim consumed it.
    fn vim_intercept(&mut self, event: &input::KeyPress) -> bool {
        use crate::vim::{VimKey, VimMode, VimOutcome};
        use winit::keyboard::{Key, NamedKey};
        let mods = self.modifiers;
        if mods.super_key() || mods.control_key() || mods.alt_key() {
            return false;
        }
        let Some(pane) = self.tabs.get_mut(self.active).and_then(|t| t.active_mut()) else {
            return false;
        };
        if !pane.editor.vim_enabled() {
            return false;
        }
        let is_escape = matches!(event.logical_key, Key::Named(NamedKey::Escape));
        // Esc closes the completion menu before it changes modes
        if self.completion.open && is_escape {
            return false;
        }
        // insert mode: only Esc is vim's; everything else keeps the normal
        // editor path (history, completions, placeholders)
        if pane.editor.vim_mode() == Some(VimMode::Insert) && !is_escape {
            return false;
        }
        let key = match &event.logical_key {
            Key::Named(NamedKey::Escape) => VimKey::Escape,
            Key::Named(NamedKey::Enter) if !mods.shift_key() => VimKey::Enter,
            Key::Named(NamedKey::Backspace) => VimKey::Backspace,
            Key::Named(NamedKey::ArrowLeft) => VimKey::Left,
            Key::Named(NamedKey::ArrowRight) => VimKey::Right,
            Key::Named(NamedKey::ArrowUp) => VimKey::Up,
            Key::Named(NamedKey::ArrowDown) => VimKey::Down,
            Key::Named(NamedKey::Space) => VimKey::Char(' '),
            Key::Character(s) => {
                let mut cs = s.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) => VimKey::Char(c),
                    _ => return false,
                }
            }
            _ => return false,
        };
        match pane.editor.vim_key(key) {
            VimOutcome::PassThrough => false,
            VimOutcome::Consumed => {
                pane.clear_selection();
                pane.selected_block = None;
                self.update_completions();
                self.blink_epoch = Instant::now();
                true
            }
            VimOutcome::Submit => {
                self.handle_editor_key(EditorKey::Submit);
                true
            }
            VimOutcome::HistoryUp => {
                self.handle_editor_key(EditorKey::HistoryUp);
                true
            }
            VimOutcome::HistoryDown => {
                self.handle_editor_key(EditorKey::HistoryDown);
                true
            }
        }
    }

    /// Terminal bell (BEL): visual flash, audible sound, tab activity.
    fn on_bell(&mut self, pane: PaneId) {
        use runway_core::Bell;
        let bell = self.config.bell;
        if matches!(bell, Bell::Visual | Bell::Both) {
            self.bell_flash = Some(Instant::now());
        }
        if matches!(bell, Bell::Audible | Bell::Both) {
            crate::notify::beep();
        }
        let active = self.active;
        if let Some((idx, tab)) = self.find_tab_of_pane(pane)
            && idx != active
        {
            tab.activity = true;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Command corrections (local, no AI): after a failed command whose
    /// command word is unknown (or a git subcommand typo), offer the closest
    /// known command in the empty editor.
    fn check_correction(&mut self, pane: PaneId) {
        let enabled = self.config.command_corrections;
        let env = self.shell_env.clone();
        let Some((_, tab)) = self.find_tab_of_pane(pane) else {
            return;
        };
        let Some(p) = tab.pane_mut(pane) else { return };
        let newest = p
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|b| matches!(b.state, BlockState::Done { .. }))
            .map(|b| (b.id, b.command.clone(), b.exit_code()));
        let Some((id, cmd, exit)) = newest else {
            return;
        };
        if p.correction_checked == Some(id) {
            return;
        }
        p.correction_checked = Some(id);
        p.correction = None;
        if !enabled || exit == Some(0) || exit.is_none() || cmd.contains('\n') {
            return;
        }
        if let Some(env) = env
            && let Some(fix) = env.suggest_correction(&cmd)
            && fix != cmd
        {
            p.correction = Some(fix);
        }
    }

    /// Desktop notification for a long command that finished while the
    /// window was not focused (Warp "notify on command completion").
    fn check_completion_notification(&mut self, pane: PaneId) {
        if !self.config.notify_on_completion {
            return;
        }
        let threshold = Duration::from_secs(self.config.notify_threshold_secs);
        let Some((_, tab)) = self.find_tab_of_pane(pane) else {
            return;
        };
        let Some(p) = tab.pane(pane) else { return };
        let done: Vec<(runway_core::BlockId, String, Option<i32>, Duration)> = p
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter_map(|b| match b.state {
                BlockState::Done { exit, duration } => {
                    Some((b.id, b.command.clone(), exit, duration))
                }
                _ => None,
            })
            .collect();
        let Some(&(newest, ..)) = done.last() else {
            return;
        };
        let seen = self.notified.get(&pane).copied();
        self.notified.insert(pane, newest);
        if self.focused {
            return;
        }
        for (id, cmd, exit, duration) in done {
            if seen.is_some_and(|s| id <= s) || duration < threshold {
                continue;
            }
            let status = match exit {
                Some(0) => "finished".to_string(),
                Some(n) => format!("failed (exit {n})"),
                None => "finished".to_string(),
            };
            crate::notify::send(
                &format!("Command {status}"),
                &format!("{cmd} · {}", terminal::format_duration(duration)),
            );
        }
    }

    /// Run every due script step (see script.rs).
    fn drive_script(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        loop {
            let now = Instant::now();
            let Some(step) = self.script.as_mut().and_then(|s| s.take_due(now)) else {
                break;
            };
            if step == crate::script::Step::WaitPrompt {
                log::debug!("script: {step:?}");
            } else {
                log::info!("script: {step:?}");
            }
            self.run_step(step, event_loop, &window);
            window.request_redraw();
        }
    }

    fn run_step(
        &mut self,
        step: crate::script::Step,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) {
        use crate::script::{Step, char_press, key_press};
        let press = |app: &mut App, chord: &str| {
            let Some((press, mods)) = key_press(chord) else {
                log::warn!("script: bad chord {chord:?}");
                return;
            };
            let saved = app.modifiers;
            app.modifiers = mods;
            app.on_key(&press, event_loop, window);
            app.modifiers = saved;
        };
        match step {
            Step::Wait(_) => {}
            Step::Key(chord) => press(self, &chord),
            Step::Type(text) => {
                for ch in text.chars() {
                    if ch == '\n' {
                        press(self, "enter");
                        continue;
                    }
                    let (p, mods) = char_press(ch);
                    let saved = self.modifiers;
                    self.modifiers = mods;
                    self.on_key(&p, event_loop, window);
                    self.modifiers = saved;
                }
            }
            Step::Shot(path) => self.capture = Some(path),
            Step::Dump(path) => {
                let text = self.dump_state();
                if let Err(e) = std::fs::write(&path, text) {
                    log::warn!("script: dump {}: {e}", path.display());
                }
            }
            Step::Resize(w, h) => {
                let _ = window.request_inner_size(LogicalSize::new(w as f32, h as f32));
            }
            Step::Move(x, y) => self.on_cursor_moved((x, y), window),
            Step::Click(x, y) => {
                self.on_cursor_moved((x, y), window);
                self.on_left(ElementState::Pressed, event_loop, window);
                self.on_left(ElementState::Released, event_loop, window);
            }
            Step::DoubleClick(x, y) => {
                self.on_cursor_moved((x, y), window);
                for _ in 0..2 {
                    self.on_left(ElementState::Pressed, event_loop, window);
                    self.on_left(ElementState::Released, event_loop, window);
                }
            }
            Step::RightClick(x, y) => {
                self.on_cursor_moved((x, y), window);
                self.on_right_press(window);
                self.on_other_button(crate::mouse::RIGHT, false, window);
            }
            Step::Drag(x1, y1, x2, y2) => {
                self.on_cursor_moved((x1, y1), window);
                self.on_left(ElementState::Pressed, event_loop, window);
                self.on_cursor_moved(((x1 + x2) / 2.0, (y1 + y2) / 2.0), window);
                self.on_cursor_moved((x2, y2), window);
                self.on_left(ElementState::Released, event_loop, window);
            }
            Step::Scroll(lines) => {
                self.on_wheel(MouseScrollDelta::LineDelta(0.0, lines / 3.0), window);
            }
            Step::Sh(cmd) => {
                let status = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .status();
                log::info!("script: sh {cmd:?} -> {status:?}");
            }
            Step::Focus(f) => {
                if self.focused != f {
                    self.report_focus(f);
                }
                self.focused = f;
            }
            Step::Ime(text) => self.commit_text(&text, event_loop, window),
            Step::Preedit(text) => self.preedit = text,
            Step::Drop(path) => self.drop_file(&path),
            Step::Mods(spec) => {
                let mut m = ModifiersState::empty();
                for part in spec.split('+').map(str::trim) {
                    match part {
                        "shift" => m |= ModifiersState::SHIFT,
                        "ctrl" => m |= ModifiersState::CONTROL,
                        "alt" | "opt" => m |= ModifiersState::ALT,
                        "cmd" => m |= ModifiersState::SUPER,
                        _ => {}
                    }
                }
                self.modifiers = m;
            }
            #[cfg(target_os = "macos")]
            Step::Menu(id) => self.run_menu_command(&id, event_loop),
            #[cfg(not(target_os = "macos"))]
            Step::Menu(id) => log::warn!("script: no menu bar on this platform ({id})"),
            Step::WaitPrompt => {
                // re-arm until the active pane shows its first prompt (or 15s)
                if !self.editor_active
                    && let Some(s) = self.script.as_mut()
                {
                    let _ = s.rearm_wait(Duration::from_millis(50), Duration::from_secs(15));
                }
            }
            Step::Quit => {
                self.save_session();
                event_loop.exit();
            }
        }
    }

    /// Textual snapshot of the app state for script assertions.
    fn dump_state(&mut self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "tabs={} active={} editor_active={} any_running={}",
            self.tabs.len(),
            self.active,
            self.editor_active,
            self.any_running
        );
        let _ = writeln!(
            out,
            "font_size={} line_height={} theme={:?} cursor={:?}",
            self.font_size, self.config.line_height, self.config.theme, self.config.cursor_style
        );
        let _ = writeln!(
            out,
            "palette={} mode={:?} query={:?} selected={} results={}",
            self.palette.open,
            self.palette.mode,
            self.palette.query,
            self.palette.selected,
            self.palette_results.len()
        );
        let _ = writeln!(
            out,
            "settings={} page={:?} search={} query={:?} matches={} current={} menu={} completion={} ghost={:?} items={}",
            self.settings.open,
            self.settings.page,
            self.search.open,
            self.search.query,
            self.search.matches.len(),
            self.search.current,
            self.context_menu.open,
            self.completion.open,
            self.completion.ghost,
            self.completion.items.len()
        );
        let _ = writeln!(
            out,
            "confirm={:?} rename={:?} filter_editing={} toast={:?} shell_env={}",
            self.confirm.as_ref().map(|c| (c.scope, c.commands.clone())),
            self.rename,
            self.filter_editing,
            self.toast.as_ref().map(|(m, _)| m.clone()),
            self.shell_env.is_some()
        );
        if self.context_menu.open {
            let labels: Vec<&str> = self.context_menu.items.iter().map(|i| i.label).collect();
            let _ = writeln!(
                out,
                "menu_items={labels:?} menu_selected={:?}",
                self.context_menu.selected
            );
        }
        for (i, tab) in self.tabs.iter().enumerate() {
            let _ = writeln!(
                out,
                "tab {i} title={:?} panes={} active_pane={} zoomed={:?} activity={} color={:?} sync={} custom_title={:?}",
                tab.display_title(),
                tab.panes.len(),
                tab.active_pane,
                tab.zoomed,
                tab.activity,
                tab.color,
                tab.sync_inputs,
                tab.custom_title
            );
            let _ = writeln!(out, "  layout={:?}", tab.root);
            for p in &tab.panes {
                let cwd = p.session.cwd.lock().unwrap().clone();
                let _ = writeln!(
                    out,
                    "  pane {} cwd={:?} editor={:?} cursor={} view_offset={} selected_block={:?} alt={} selection={} last_exit={:?} vim={:?} git={:?}",
                    p.session.id,
                    cwd,
                    p.editor.buffer,
                    p.editor.cursor,
                    p.view_offset,
                    p.selected_block,
                    p.alt_screen,
                    p.has_selection,
                    p.last_exit,
                    p.editor.vim_mode(),
                    p.git.as_ref().map(|g| g.branch.clone())
                );
                let _ = writeln!(
                    out,
                    "    editor_selection={:?} filter={:?} correction={:?}",
                    p.editor.selected_text(),
                    p.filter,
                    p.correction
                );
                let blocks = p.session.blocks.lock().unwrap().snapshot();
                for b in blocks {
                    let _ = writeln!(
                        out,
                        "    block {} state={} cmd={:?} exit={:?} collapsed={} bookmarked={} prompt_start={} output_start={:?} end={:?} cwd={:?}",
                        b.id,
                        match b.state {
                            runway_core::BlockState::Prompt => "prompt",
                            runway_core::BlockState::Running { .. } => "running",
                            runway_core::BlockState::Done { .. } => "done",
                        },
                        b.command,
                        b.exit_code(),
                        b.collapsed,
                        b.bookmarked,
                        b.prompt_start,
                        b.output_start,
                        b.end,
                        b.cwd
                    );
                }
                // visible rows (what the user sees), for text assertions
                let text = crate::views::terminal::visible_text(&p.session);
                for line in text.lines() {
                    let _ = writeln!(out, "    | {line}");
                }
            }
        }
        let clip = self
            .clipboard
            .as_mut()
            .and_then(|c| c.get_text().ok())
            .unwrap_or_default();
        let _ = writeln!(out, "clipboard={clip:?}");
        out
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        self.config = Config::load();
        let restored = if self.config.restore_session && self.persist_session {
            runway_core::session_state::load()
        } else {
            None
        };
        // last window frame (only when it is still a sane size)
        let frame = restored
            .as_ref()
            .and_then(|s| s.window)
            .filter(|f| f.width >= 320.0 && f.height >= 200.0);
        let mut attrs = WindowAttributes::default()
            .with_title("runway")
            // always transparent so `window_opacity` can change live
            .with_transparent(true)
            .with_inner_size(match frame {
                Some(f) => LogicalSize::new(f.width, f.height),
                None => LogicalSize::new(1180.0, 760.0),
            });
        if let Some(f) = frame {
            attrs = attrs
                .with_position(winit::dpi::LogicalPosition::new(f.x, f.y))
                .with_maximized(f.maximized);
            if f.fullscreen {
                attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
            }
        }

        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs = attrs
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true);
        }

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        // input methods (CJK composition, dead keys, emoji picker)
        window.set_ime_allowed(true);
        self.os_light = window.theme() == Some(winit::window::Theme::Light);
        let size = window.inner_size();
        let gpu = Gpu::new(window.clone(), size.width, size.height);
        let quads = QuadPipeline::new(&gpu.device, gpu.config.format);
        self.bg_image = Some(runway_renderer::ImagePipeline::new(
            &gpu.device,
            gpu.config.format,
        ));
        let mut text = TextEngine::new(&gpu.device, &gpu.queue, gpu.config.format);
        self.cell_w = text.mono_cell_width(self.config.font_size);

        let git_proxy = self.proxy.clone();
        self.git = Some(GitWorker::new(Arc::new(move |pane, info| {
            let _ = git_proxy.send_event(UserEvent::GitStatus(pane, info));
        })));

        let watch_proxy = self.proxy.clone();
        self._watcher = runway_core::watch::watch(Arc::new(move || {
            let _ = watch_proxy.send_event(UserEvent::ConfigReloaded);
        }));

        #[cfg(target_os = "macos")]
        {
            let menu_proxy = self.proxy.clone();
            muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
                let _ = menu_proxy.send_event(UserEvent::Menu(e.id.0));
            }));
        }

        self.window = Some(window);
        self.gpu = Some(gpu);
        self.quads = Some(quads);
        self.text = Some(text);

        self.apply_config();
        self.workflows = runway_core::workflows::load_all();
        self.launch_configs = runway_core::launch::load_all();

        let hist_proxy = self.proxy.clone();
        std::thread::Builder::new()
            .name("history-import".into())
            .spawn(move || {
                let h = runway_core::history::load_shell_history(5000);
                let _ = hist_proxy.send_event(UserEvent::HistoryLoaded(h));
            })
            .ok();

        match restored {
            Some(saved) if !saved.tabs.is_empty() => {
                // restoring must not rewrite the file tab by tab
                self.persist_session = false;
                for tab in &saved.tabs {
                    self.restore_tab(tab);
                }
                self.persist_session = true;
                self.active = saved.active.min(self.tabs.len() - 1);
                self.save_session();
            }
            _ => self.spawn_tab(),
        }
        self.load_shell_env();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => {
                self.request_close(CloseScope::Window, event_loop);
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                window.request_redraw();
            }
            WindowEvent::ThemeChanged(t) => {
                self.os_light = t == winit::window::Theme::Light;
                if self.config.sync_os_theme {
                    self.apply_config();
                }
            }
            WindowEvent::Focused(f) => {
                if self.focused != f {
                    self.report_focus(f);
                }
                self.focused = f;
                window.request_redraw();
            }
            WindowEvent::Ime(ime) => match ime {
                winit::event::Ime::Preedit(text, _) => {
                    self.preedit = text;
                    window.request_redraw();
                }
                winit::event::Ime::Commit(text) => self.commit_text(&text, event_loop, &window),
                winit::event::Ime::Enabled | winit::event::Ime::Disabled => {
                    self.preedit.clear();
                    window.request_redraw();
                }
            },
            WindowEvent::DroppedFile(path) => {
                self.drop_file(&path);
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                self.update_cursor_icon(&window);
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                // the input method owns keys while it is composing
                if !self.preedit.is_empty() {
                    return;
                }
                let press = input::KeyPress::from(&event);
                self.on_key(&press, event_loop, &window);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = window.scale_factor() as f32;
                let pos = (position.x as f32 / scale, position.y as f32 / scale);
                self.on_cursor_moved(pos, &window);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Right,
                ..
            } => self.on_right_press(&window),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: winit::event::MouseButton::Right,
                ..
            } => self.on_other_button(crate::mouse::RIGHT, false, &window),
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Middle,
                ..
            } => self.on_other_button(
                crate::mouse::MIDDLE,
                state == ElementState::Pressed,
                &window,
            ),
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Left,
                ..
            } => self.on_left(state, event_loop, &window),
            WindowEvent::MouseWheel { delta, .. } => self.on_wheel(delta, &window),
            WindowEvent::RedrawRequested => {
                self.redraw(&window);
                if self.any_running {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::PtyWakeup(pane) => {
                self.flush_pending_commands(pane);
                self.check_completion_notification(pane);
                self.check_correction(pane);
                let active_idx = self.active;
                if let Some((idx, tab)) = self.find_tab_of_pane(pane) {
                    if idx == active_idx {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    } else {
                        tab.activity = true;
                        if let Some(p) = tab.pane(pane) {
                            p.session
                                .dirty
                                .store(false, std::sync::atomic::Ordering::Release);
                        }
                        if let Some(w) = &self.window {
                            w.request_redraw(); // tab bar dot
                        }
                    }
                }
            }
            UserEvent::PtyTitle(pane, title) => {
                if let Some((_, tab)) = self.find_tab_of_pane(pane) {
                    tab.title = title;
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::PtyBell(pane) => {
                self.on_bell(pane);
            }
            UserEvent::GlobalHotkey => self.toggle_visibility(),
            #[cfg(target_os = "macos")]
            UserEvent::Menu(id) => self.run_menu_command(&id, event_loop),
            #[cfg(not(target_os = "macos"))]
            UserEvent::Menu(_) => {}
            UserEvent::PtyClipboard(text) => {
                self.copy_to_clipboard(text);
            }
            UserEvent::PtyColorRequest(pane, index, reply) => {
                // 0–255 palette, 256 foreground, 257 background, 258 cursor
                let color = match index {
                    0..=255 => Some(terminal::indexed_color(&self.theme, index as u8)),
                    256 => Some(self.theme.fg),
                    257 => Some(self.theme.bg.with_alpha(1.0)),
                    258 => Some(self.theme.cursor),
                    _ => None,
                };
                if let Some(c) = color
                    && let Some((_, tab)) = self.find_tab_of_pane(pane)
                    && let Some(p) = tab.pane(pane)
                {
                    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    let rgb = runway_pty::alacritty_terminal::vte::ansi::Rgb {
                        r: to8(c.r),
                        g: to8(c.g),
                        b: to8(c.b),
                    };
                    p.session.write((reply.0)(rgb).as_bytes());
                }
            }
            UserEvent::PtyExit(pane) => {
                self.close_pane(pane, event_loop);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::ShellEnvLoaded(env) => {
                self.shell_env = Some(env);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            UserEvent::HistoryLoaded(mut history) => {
                log::info!("shell history imported: {} entries", history.len());
                self.shell_history.append(&mut history);
            }
            UserEvent::ConfigReloaded => {
                let fresh = Config::load();
                if fresh != self.config {
                    self.config = fresh;
                    self.apply_config();
                }
            }
            UserEvent::GitStatus(pane, info) => {
                if let Some((_, tab)) = self.find_tab_of_pane(pane)
                    && let Some(p) = tab.pane_mut(pane)
                {
                    p.git = info;
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.script.is_some() {
            self.drive_script(event_loop);
            if let Some(due) = self.script.as_ref().and_then(|s| s.due()) {
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(due));
                return;
            }
        }
        // repaint once more when a toast expires
        if let Some((_, at)) = &self.toast {
            let until = *at + TOAST_TTL;
            if Instant::now() >= until {
                self.toast = None;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            } else if !self.editor_active && !self.palette.open && !self.any_running {
                event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(until));
                return;
            }
        }
        if (self.editor_active || self.palette.open || self.rename.is_some()) && !self.any_running {
            let elapsed = self.blink_epoch.elapsed().as_millis();
            let next_flip = BLINK_INTERVAL.as_millis() - (elapsed % BLINK_INTERVAL.as_millis());
            event_loop.set_control_flow(winit::event_loop::ControlFlow::wait_duration(
                Duration::from_millis(next_flip as u64 + 5),
            ));
            if next_flip > BLINK_INTERVAL.as_millis() - 30
                && let Some(w) = &self.window
            {
                w.request_redraw();
            }
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }
}
