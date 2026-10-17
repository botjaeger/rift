use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

use crate::input::AppAction;
use crate::state::{ClickTarget, Hit};
use crate::views::icons::{self, Icon};

/// What running a palette row does.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteAction {
    App(AppAction),
    OpenSettings,
    SwitchTheme(usize),
    RunHistory(String),
    /// ⌃R history search: Enter inserts into the editor instead of running.
    InsertCommand(String),
    /// Index into the app's loaded launch configurations.
    Launch(usize),
    ToggleVim,
    Search,
    OpenHistory,
    /// Session switcher: activate tab `.0`.
    FocusTab(usize),
    /// Session switcher: focus pane `.1` of tab `.0`.
    FocusPane(usize, rift_pty::PaneId),
    /// Session switcher: show block `.2` of pane `.1` in tab `.0`.
    ShowBlock(usize, rift_pty::PaneId, rift_core::BlockId),
    /// Pipe mode: run the typed command with the block output on stdin.
    RunPipe,
    /// Open a new tab connected to ssh profile `.0`.
    Ssh(usize),
    /// Open a tab reattached to session-service shell `.0`.
    AttachService(u64),
    /// Session switcher: raise window `.0` (pid) at tab `.1`, pane `.2`.
    FocusWindow(u32, usize, u64),
    /// Ask to install remote shell integration on ssh profile `.0`.
    SshInstall(usize),
}

/// Palette rows for ssh profiles: connect, and install integration.
pub fn ssh_items(profiles: &[rift_core::ssh::SshProfile]) -> Vec<Item> {
    let mut out = Vec::new();
    for (i, p) in profiles.iter().enumerate() {
        out.push(Item {
            title: format!("SSH: {}", p.name),
            id: if p.name == p.host {
                String::new()
            } else {
                p.host.clone()
            },
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::Ssh(i),
            mono_title: false,
        });
    }
    for (i, p) in profiles.iter().enumerate() {
        out.push(Item {
            title: format!("Install Rift shell integration on {}", p.name),
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Settings,
            action: PaletteAction::SshInstall(i),
            mono_title: false,
        });
    }
    out
}

/// What the modal is searching over (Warp parity: ⌘K / ⌃R).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PaletteMode {
    #[default]
    Commands,
    History,
    /// ⇧⌘O: tabs, panes and recent blocks of this window.
    Sessions,
    /// Block action: the query is a command; the rows preview the output
    /// that will be its stdin.
    PipeOutput,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Section {
    Actions,
    Settings,
    Recent,
    Tabs,
    Panes,
    Blocks,
    Preview,
}

#[derive(Clone)]
pub struct Item {
    pub title: String,
    pub id: String,
    pub shortcut: Vec<String>,
    pub section: Section,
    pub action: PaletteAction,
    pub mono_title: bool,
}

pub struct MatchedItem {
    pub item: Item,
    pub indices: Vec<u32>,
    pub score: u32,
}

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub mode: PaletteMode,
}

/// App actions, settings and dynamic launch/theme entries.
pub fn base_items(
    theme_names: &[String],
    launch_names: &[String],
    keymap: &rift_core::Keymap,
) -> Vec<Item> {
    let shortcuts: std::collections::BTreeMap<String, Vec<String>> =
        keymap.display_rows().into_iter().collect();
    let item = |title: &str, id: &str, action: PaletteAction| Item {
        title: title.into(),
        id: id.into(),
        shortcut: shortcuts.get(id).cloned().unwrap_or_default(),
        section: Section::Actions,
        action,
        mono_title: false,
    };

    let mut items = vec![
        item("Search in terminal", "search", PaletteAction::Search),
        item("Command history", "history", PaletteAction::OpenHistory),
        item(
            "Go to tab, pane or block",
            "session_switcher",
            PaletteAction::App(AppAction::SessionSwitcher),
        ),
        item(
            "Notifications: finished commands and alerts",
            "attention_center",
            PaletteAction::App(AppAction::AttentionCenter),
        ),
        item(
            "Show link and path hints",
            "hint_mode",
            PaletteAction::App(AppAction::HintMode),
        ),
        item(
            "Send block output to a command",
            "pipe_block_output",
            PaletteAction::App(AppAction::PipeBlockOutput),
        ),
        item(
            "Toggle vertical tabs",
            "toggle_tab_layout",
            PaletteAction::App(AppAction::ToggleTabLayout),
        ),
        item(
            "Clear saved block transcripts",
            "clear_transcripts",
            PaletteAction::App(AppAction::ClearTranscripts),
        ),
        item("New tab", "new_tab", PaletteAction::App(AppAction::NewTab)),
        item(
            "Reopen closed tab",
            "reopen_closed_tab",
            PaletteAction::App(AppAction::ReopenClosedTab),
        ),
        item(
            "Close pane",
            "close_pane",
            PaletteAction::App(AppAction::ClosePane),
        ),
        item(
            "Next tab",
            "tab_next",
            PaletteAction::App(AppAction::TabNext),
        ),
        item(
            "Previous tab",
            "tab_prev",
            PaletteAction::App(AppAction::TabPrev),
        ),
        item(
            "Move tab left",
            "move_tab_left",
            PaletteAction::App(AppAction::MoveTabLeft),
        ),
        item(
            "Move tab right",
            "move_tab_right",
            PaletteAction::App(AppAction::MoveTabRight),
        ),
        item(
            "Split pane right",
            "split_vertical",
            PaletteAction::App(AppAction::SplitRight),
        ),
        item(
            "Split pane down",
            "split_horizontal",
            PaletteAction::App(AppAction::SplitDown),
        ),
        item(
            "Focus pane left",
            "navigate_left",
            PaletteAction::App(AppAction::FocusLeft),
        ),
        item(
            "Focus pane right",
            "navigate_right",
            PaletteAction::App(AppAction::FocusRight),
        ),
        item(
            "Focus pane above",
            "navigate_up",
            PaletteAction::App(AppAction::FocusUp),
        ),
        item(
            "Focus pane below",
            "navigate_down",
            PaletteAction::App(AppAction::FocusDown),
        ),
        item(
            "Maximize active pane",
            "maximize_pane",
            PaletteAction::App(AppAction::MaximizePane),
        ),
        item(
            "Resize pane left",
            "resize_pane_left",
            PaletteAction::App(AppAction::ResizePaneLeft),
        ),
        item(
            "Resize pane right",
            "resize_pane_right",
            PaletteAction::App(AppAction::ResizePaneRight),
        ),
        item(
            "Resize pane up",
            "resize_pane_up",
            PaletteAction::App(AppAction::ResizePaneUp),
        ),
        item(
            "Resize pane down",
            "resize_pane_down",
            PaletteAction::App(AppAction::ResizePaneDown),
        ),
        item(
            "Equalize pane sizes",
            "equalize_panes",
            PaletteAction::App(AppAction::EqualizePanes),
        ),
        item(
            "Focus terminal input",
            "focus_input",
            PaletteAction::App(AppAction::FocusInput),
        ),
        item(
            "Select previous block",
            "select_block_prev",
            PaletteAction::App(AppAction::SelectBlockPrev),
        ),
        item(
            "Select next block",
            "select_block_next",
            PaletteAction::App(AppAction::SelectBlockNext),
        ),
        item(
            "Toggle block bookmark",
            "bookmark_toggle",
            PaletteAction::App(AppAction::BookmarkToggle),
        ),
        item(
            "Previous bookmark",
            "bookmark_prev",
            PaletteAction::App(AppAction::BookmarkPrev),
        ),
        item(
            "Next bookmark",
            "bookmark_next",
            PaletteAction::App(AppAction::BookmarkNext),
        ),
        item(
            "Copy block command",
            "copy_block_command",
            PaletteAction::App(AppAction::CopyBlockCommand),
        ),
        item(
            "Copy block output",
            "copy_block_output",
            PaletteAction::App(AppAction::CopyBlockOutput),
        ),
        item(
            "Reinput selected command",
            "reinput",
            PaletteAction::App(AppAction::Reinput),
        ),
        item(
            "Open block actions",
            "block_menu",
            PaletteAction::App(AppAction::BlockMenu),
        ),
        item(
            "Clear all blocks",
            "clear_blocks",
            PaletteAction::App(AppAction::ClearBlocks),
        ),
        item(
            "Scroll one page up",
            "scroll_page_up",
            PaletteAction::App(AppAction::ScrollPageUp),
        ),
        item(
            "Scroll one page down",
            "scroll_page_down",
            PaletteAction::App(AppAction::ScrollPageDown),
        ),
        item(
            "Scroll to top",
            "scroll_top",
            PaletteAction::App(AppAction::ScrollTop),
        ),
        item(
            "Scroll to bottom",
            "scroll_bottom",
            PaletteAction::App(AppAction::ScrollBottom),
        ),
        item(
            "Increase font size",
            "font_increase",
            PaletteAction::App(AppAction::FontIncrease),
        ),
        item(
            "Decrease font size",
            "font_decrease",
            PaletteAction::App(AppAction::FontDecrease),
        ),
        item(
            "Reset font size",
            "font_reset",
            PaletteAction::App(AppAction::FontReset),
        ),
        item(
            "Filter block output",
            "filter_block",
            PaletteAction::App(AppAction::FilterBlock),
        ),
        item(
            "Select all",
            "select_all",
            PaletteAction::App(AppAction::SelectAll),
        ),
        item(
            "Rename tab",
            "rename_tab",
            PaletteAction::App(AppAction::RenameTab),
        ),
        item(
            "Close other tabs",
            "close_other_tabs",
            PaletteAction::App(AppAction::CloseOtherTabs),
        ),
        item(
            "Reset time on this tab",
            "reset_tab_timer",
            PaletteAction::App(AppAction::ResetTabTimer),
        ),
        item(
            "Toggle synchronized inputs in tab",
            "toggle_sync_inputs",
            PaletteAction::App(AppAction::ToggleSyncInputs),
        ),
        item(
            "Save current layout as launch configuration",
            "save_launch_config",
            PaletteAction::App(AppAction::SaveLaunchConfig),
        ),
        item(
            "New window",
            "new_window",
            PaletteAction::App(AppAction::NewWindow),
        ),
        item(
            "Toggle fullscreen",
            "toggle_fullscreen",
            PaletteAction::App(AppAction::ToggleFullscreen),
        ),
        item(
            "Minimize window",
            "minimize",
            PaletteAction::App(AppAction::Minimize),
        ),
        item("Quit Rift", "quit", PaletteAction::App(AppAction::Quit)),
        item(
            "Open settings",
            "open_settings",
            PaletteAction::OpenSettings,
        ),
        item(
            "Open keyboard shortcuts",
            "open_keybindings",
            PaletteAction::App(AppAction::OpenKeybindings),
        ),
        item(
            "About Rift",
            "open_about",
            PaletteAction::App(AppAction::OpenAbout),
        ),
        item(
            "Check for updates",
            "check_for_updates",
            PaletteAction::App(AppAction::CheckForUpdates),
        ),
        item(
            "Update Rift",
            "update_rift",
            PaletteAction::App(AppAction::UpdateRift),
        ),
        item(
            "What's new",
            "whats_new",
            PaletteAction::App(AppAction::WhatsNew),
        ),
        item(
            "Toggle file explorer",
            "toggle_explorer",
            PaletteAction::App(AppAction::ToggleExplorer),
        ),
        item(
            "Toggle code review",
            "toggle_review",
            PaletteAction::App(AppAction::ToggleReview),
        ),
    ];
    items.push(Item {
        title: "Toggle vim keybindings".into(),
        id: "toggle_vim".into(),
        shortcut: Vec::new(),
        section: Section::Settings,
        action: PaletteAction::ToggleVim,
        mono_title: false,
    });
    for (i, name) in launch_names.iter().enumerate() {
        items.push(Item {
            title: format!("Launch configuration: {name}"),
            id: "launch".into(),
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::Launch(i),
            mono_title: false,
        });
    }
    for (i, name) in theme_names.iter().enumerate() {
        items.push(Item {
            title: format!("Theme: {name}"),
            id: "set_theme".into(),
            shortcut: Vec::new(),
            section: Section::Settings,
            action: PaletteAction::SwitchTheme(i),
            mono_title: false,
        });
    }
    items
}

/// ⌃R mode: every history entry as an insertable row (newest first).
pub fn history_items(history: &[String]) -> Vec<Item> {
    history
        .iter()
        .rev()
        .take(2000)
        .map(|h| Item {
            title: h.clone(),
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Recent,
            action: PaletteAction::InsertCommand(h.clone()),
            mono_title: true,
        })
        .collect()
}

/// Pipe mode rows: what runs, then a preview of the stdin text.
pub fn pipe_items(query: &str, output: &str) -> Vec<MatchedItem> {
    let lines: Vec<&str> = output.lines().collect();
    let row = |title: String, section: Section, mono_title: bool| MatchedItem {
        item: Item {
            title,
            id: String::new(),
            shortcut: Vec::new(),
            section,
            action: PaletteAction::RunPipe,
            mono_title,
        },
        indices: Vec::new(),
        score: 0,
    };
    let head = if query.trim().is_empty() {
        format!("Type a command to receive {} lines on stdin", lines.len())
    } else {
        format!("Run `{}` with {} lines on stdin", query.trim(), lines.len())
    };
    let mut out = vec![row(head, Section::Actions, false)];
    for l in lines.iter().take(6) {
        out.push(row(l.to_string(), Section::Preview, true));
    }
    if lines.len() > 6 {
        out.push(row(
            format!("… {} more lines", lines.len() - 6),
            Section::Preview,
            false,
        ));
    }
    out
}

/// The shell command that runs `command` with `file` as its stdin. The
/// output is never interpolated into the command line, only the (quoted)
/// path of a private file holding it.
pub fn pipe_command(command: &str, file: &str, shell: &str) -> String {
    let quoted = crate::input::shell_quote(file);
    if shell.ends_with("fish") {
        format!("begin; {command}; end < {quoted}")
    } else {
        format!("( {command} ) < {quoted}")
    }
}

/// Fuzzy-rank items + recent history against the query.
pub fn rank(items: &[Item], history: &[String], query: &str) -> Vec<MatchedItem> {
    let mut all: Vec<Item> = items.to_vec();
    for h in history.iter().rev().take(20) {
        all.push(Item {
            title: h.clone(),
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Recent,
            action: PaletteAction::RunHistory(h.clone()),
            mono_title: true,
        });
    }

    if query.is_empty() {
        return all
            .into_iter()
            .map(|item| MatchedItem {
                item,
                indices: vec![],
                score: 0,
            })
            .collect();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut out: Vec<MatchedItem> = all
        .into_iter()
        .filter_map(|item| {
            let mut buf = Vec::new();
            let hay = Utf32Str::new(&item.title, &mut buf);
            let mut indices = Vec::new();
            pattern
                .indices(hay, &mut matcher, &mut indices)
                .map(|score| MatchedItem {
                    item,
                    indices,
                    score,
                })
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.score));
    out
}

const MODAL_W: f32 = 640.0;
const ROW_H: f32 = 36.0;
const SECTION_H: f32 = 28.0;
/// Height of the "No matches" band shown for an empty result list.
const EMPTY_H: f32 = 56.0;
const SEARCH_H: f32 = 48.0;
const MODES_H: f32 = 40.0;
const FOOTER_H: f32 = 34.0;

/// The palette's switchable modes, their chip labels and typed prefixes.
pub const MODES: [(PaletteMode, &str, char); 3] = [
    (PaletteMode::Commands, "Actions", '>'),
    (PaletteMode::History, "History", '#'),
    (PaletteMode::Sessions, "Sessions", '@'),
];

/// The mode a typed prefix switches to (`#` history, `@` sessions, `>`
/// actions).
pub fn mode_for_prefix(c: char) -> Option<PaletteMode> {
    MODES.iter().find(|m| m.2 == c).map(|m| m.0)
}

/// ⇥: the next switchable mode, wrapping.
pub fn next_mode(mode: PaletteMode) -> PaletteMode {
    let i = MODES.iter().position(|m| m.0 == mode).unwrap_or(0);
    MODES[(i + 1) % MODES.len()].0
}

pub(crate) fn sans(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    buf.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buf
}

fn mono(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    buf.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new()
            .family(Family::Name(mono_family()))
            .weight(weight),
        Shaping::Advanced,
        None,
    );
    buf
}

pub(crate) fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// Render the palette modal over a dimmed backdrop: the query, the mode
/// chips, grouped rows with icons and shortcut caps, and a key-hint footer.
/// Row targets go to `row_hits` as (row_rect, index); mode chips to `hits`.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &PaletteState,
    results: &[MatchedItem],
    w: f32,
    h: f32,
    blink_on: bool,
    hits: &mut Vec<Hit>,
    row_hits: &mut Vec<([f32; 4], usize)>,
) {
    layer.rect([0.0, 0.0], [w, h], Color::BLACK.with_alpha(0.4), 0.0);

    // the modal narrows with the window (16px margin each side)
    let modal_w = MODAL_W.min((w - 32.0).max(160.0));
    let mx = (w - modal_w) / 2.0;
    let my = crate::views::tabbar::TAB_BAR_H + 54.0;
    let modes_h = if state.mode == PaletteMode::PipeOutput {
        0.0
    } else {
        MODES_H
    };

    // Keep the keyboard-selected row visible when the result list is longer
    // than the modal. Row hit indices remain absolute into `results`.
    // (fewer rows when the window is short: the modal must fit under the tab bar)
    let room = h - my - SEARCH_H - modes_h - FOOTER_H - 14.0 - 8.0 - SECTION_H;
    let max_rows = ((room / ROW_H).max(0.0) as usize).clamp(1, 9);
    let shown = results.len().min(max_rows);
    let start = state
        .selected
        .saturating_add(1)
        .saturating_sub(max_rows)
        .min(results.len().saturating_sub(shown));
    let visible = &results[start..start + shown];
    let mut list_h = 6.0 * 2.0;
    let mut last_section = None;
    for m in visible {
        if last_section != Some(m.item.section) {
            list_h += SECTION_H;
        }
        last_section = Some(m.item.section);
        list_h += ROW_H;
    }
    // no results: a short "No matches" band instead of a blank gap
    if visible.is_empty() {
        list_h += EMPTY_H;
    }
    let modal_h = SEARCH_H + modes_h + list_h + FOOTER_H;

    layer.shadow(
        [mx, my + 12.0],
        [modal_w, modal_h],
        Color::BLACK.with_alpha(0.55),
        8.0,
        24.0,
    );
    layer.bordered(
        [mx, my],
        [modal_w, modal_h],
        theme.surface,
        lift(theme, 0.1),
        8.0,
        1.0,
    );
    layer.set_clip(Some((
        [mx + 1.0, my + 1.0, modal_w - 2.0, modal_h - 2.0],
        7.0,
    )));

    // ── search row: magnifier, query, esc ──
    let cy = my + SEARCH_H / 2.0;
    icons::draw(
        layer,
        Icon::Search,
        [mx + 16.0 + 8.0, cy],
        16.0,
        theme.text_dim,
    );
    let qx = mx + 16.0 + 16.0 + 10.0;
    if !state.query.is_empty() {
        layer.text(
            sans(engine, &state.query, 15.0, Weight::NORMAL),
            qx,
            cy - 10.0,
            theme.text_bright,
        );
    } else {
        let placeholder = match state.mode {
            PaletteMode::Sessions => "Go to tab, pane or block…",
            PaletteMode::PipeOutput => "Command that reads the output on stdin…",
            PaletteMode::History => "Search command history…",
            PaletteMode::Commands => "Search actions, or type # @ to switch…",
        };
        layer.text(
            sans(engine, placeholder, 15.0, Weight::NORMAL),
            qx + 4.0,
            cy - 10.0,
            theme.text_muted,
        );
    }
    if blink_on {
        let qw = engine.measure(&state.query, 15.0, FONT_SANS, Weight::NORMAL);
        layer.rect([qx + qw + 1.0, cy - 9.0], [2.0, 18.0], theme.accent, 0.0);
    }
    // result count, then the esc cap
    let esc_w = (engine.measure("esc", 10.5, FONT_SANS, Weight::MEDIUM) + 10.0).max(18.0);
    let esc_x = mx + modal_w - 16.0 - esc_w;
    super::completion::key_cap(layer, engine, theme, "esc", esc_x, cy);
    let count = format!("{} results", results.len());
    let cw = engine.measure(&count, 11.5, FONT_SANS, Weight::NORMAL);
    let qw_now = engine.measure(&state.query, 15.0, FONT_SANS, Weight::NORMAL);
    if qx + qw_now + 24.0 < esc_x - 12.0 - cw {
        layer.text(
            sans(engine, &count, 11.5, Weight::NORMAL),
            esc_x - 12.0 - cw,
            cy - 7.8,
            theme.text_muted,
        );
    }
    layer.hline(mx + 1.0, my + SEARCH_H, modal_w - 2.0, 1.0, theme.border);

    // ── mode chips ──
    if modes_h > 0.0 {
        let ccy = my + SEARCH_H + MODES_H / 2.0;
        let mut cx = mx + 12.0;
        for (i, (mode, label, prefix)) in MODES.iter().enumerate() {
            let on = state.mode == *mode;
            let lw = engine.measure(label, 12.0, FONT_SANS, Weight::MEDIUM);
            let pw = engine.measure(&prefix.to_string(), 11.0, mono_family(), Weight::NORMAL);
            let r = [cx, ccy - 12.0, 10.0 + lw + 6.0 + pw + 10.0, 24.0];
            if on {
                layer.bordered(
                    [r[0], r[1]],
                    [r[2], r[3]],
                    lift(theme, 0.08),
                    lift(theme, 0.1),
                    4.0,
                    1.0,
                );
            }
            layer.text(
                sans(engine, label, 12.0, Weight::MEDIUM),
                r[0] + 10.0,
                ccy - 8.1,
                if on {
                    theme.text_bright
                } else {
                    theme.text_dim
                },
            );
            layer.text(
                mono(engine, &prefix.to_string(), 11.0, Weight::NORMAL),
                r[0] + 10.0 + lw + 6.0,
                ccy - 7.4,
                theme.text_muted,
            );
            hits.push(Hit {
                rect: r,
                target: ClickTarget::PaletteMode(i as u8),
            });
            cx += r[2] + 4.0;
        }
        layer.hline(
            mx + 1.0,
            my + SEARCH_H + MODES_H,
            modal_w - 2.0,
            1.0,
            theme.border,
        );
    }

    // ── results ──
    let mut y = my + SEARCH_H + modes_h + 6.0;
    if visible.is_empty() {
        let msg = "No matches";
        let mw = engine.measure(msg, 13.0, FONT_SANS, Weight::NORMAL);
        layer.text(
            sans(engine, msg, 13.0, Weight::NORMAL),
            mx + (modal_w - mw) / 2.0,
            y + (EMPTY_H - 17.5) / 2.0,
            theme.text_muted,
        );
    }
    let mut last_section = None;
    for (visible_i, m) in visible.iter().enumerate() {
        let i = start + visible_i;
        if last_section != Some(m.item.section) {
            let label = match m.item.section {
                Section::Actions => match state.mode {
                    PaletteMode::History => "HISTORY",
                    _ => "ACTIONS",
                },
                Section::Settings => "SETTINGS",
                Section::Recent => "RECENT COMMANDS",
                Section::Tabs => "TABS",
                Section::Panes => "PANES",
                Section::Blocks => "RECENT BLOCKS",
                Section::Preview => "OUTPUT (STDIN)",
            };
            let mut lb = Buffer::new(&mut engine.font_system, Metrics::new(11.0, 14.0));
            lb.set_text(
                &mut engine.font_system,
                label,
                &Attrs::new()
                    .family(Family::Name(FONT_SANS))
                    .weight(Weight::SEMIBOLD)
                    .letter_spacing(0.06),
                Shaping::Advanced,
                None,
            );
            layer.text(lb, mx + 16.0, y + 10.0, theme.text_muted);
            y += SECTION_H;
        }
        last_section = Some(m.item.section);

        let row_rect = [mx + 6.0, y, modal_w - 12.0, ROW_H];
        let selected = i == state.selected;
        if selected {
            layer.rect(
                [row_rect[0], row_rect[1]],
                [row_rect[2], row_rect[3]],
                theme.accent.with_alpha(0.16),
                4.0,
            );
        }
        row_hits.push((row_rect, i));

        let rcy = y + ROW_H / 2.0;
        let icon_color = if selected {
            theme.text_ui
        } else {
            theme.text_dim
        };
        icons::draw(
            layer,
            item_icon(&m.item),
            [row_rect[0] + 10.0 + 8.0, rcy],
            16.0,
            icon_color,
        );

        // title with match highlighting
        let tx = row_rect[0] + 10.0 + 16.0 + 12.0;
        let title_color = if selected {
            theme.text_bright
        } else {
            theme.text_ui
        };
        let (size, family) = if m.item.mono_title {
            (12.5, mono_family())
        } else {
            (13.5, FONT_SANS)
        };
        // everything left of the shortcut caps must fit the row
        let caps_w: f32 = m
            .item
            .shortcut
            .iter()
            .map(|k| (engine.measure(k, 10.5, FONT_SANS, Weight::MEDIUM) + 10.0).max(18.0) + 4.0)
            .sum();
        let max_right = mx + modal_w - 16.0 - caps_w - 8.0;
        // a long title keeps at least 55% of the row; the detail gets the rest
        let avail = max_right - tx;
        let id_w = if m.item.id.is_empty() {
            0.0
        } else {
            engine.measure(&m.item.id, 11.0, mono_family(), Weight::NORMAL) + 12.0
        };
        let full_w = engine.measure(&m.item.title, size, family, Weight::NORMAL);
        let budget = if full_w + id_w <= avail {
            avail
        } else {
            (avail - id_w).max(avail * 0.55)
        };
        let title = elide_to(engine, &m.item.title, size, family, budget);
        let mut tbuf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
        let chars: Vec<char> = title.chars().collect();
        let mut spans: Vec<(String, bool)> = Vec::new();
        for (ci, ch) in chars.iter().enumerate() {
            let matched = m.indices.contains(&(ci as u32));
            match spans.last_mut() {
                Some((s, was)) if *was == matched => s.push(*ch),
                _ => spans.push((ch.to_string(), matched)),
            }
        }
        let rich: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|(s, matched)| {
                let attrs = Attrs::new().family(Family::Name(family));
                if *matched {
                    (
                        s.as_str(),
                        attrs
                            .weight(Weight::SEMIBOLD)
                            .color(text_color(theme.accent)),
                    )
                } else {
                    (s.as_str(), attrs.color(text_color(title_color)))
                }
            })
            .collect();
        tbuf.set_rich_text(
            &mut engine.font_system,
            rich,
            &Attrs::new().family(Family::Name(family)),
            Shaping::Advanced,
            None,
        );
        let title_w = engine.measure(&title, size, family, Weight::NORMAL);
        layer.text(tbuf, tx, y + (ROW_H - size * 1.35) / 2.0, title_color);

        // action id / detail, in whatever room is left
        let id_x = tx + title_w + 12.0;
        if !m.item.id.is_empty() && max_right - id_x > 30.0 {
            let id = elide_to(engine, &m.item.id, 11.0, mono_family(), max_right - id_x);
            layer.text(
                mono(engine, &id, 11.0, Weight::NORMAL),
                id_x,
                y + (ROW_H - 14.85) / 2.0,
                theme.text_muted,
            );
        }

        // shortcut caps (right-aligned)
        let mut kx = mx + modal_w - 16.0 - caps_w + 4.0;
        for key in &m.item.shortcut {
            kx += super::completion::key_cap(layer, engine, theme, key, kx, rcy) + 4.0;
        }

        y += ROW_H;
    }

    // ── footer ──
    let fy = my + modal_h - FOOTER_H;
    layer.rect(
        [mx, fy],
        [modal_w, FOOTER_H],
        Color::BLACK.with_alpha(0.14),
        0.0,
    );
    layer.hline(mx + 1.0, fy, modal_w - 2.0, 1.0, theme.border);
    let enter = match state.mode {
        PaletteMode::Sessions => "go to",
        PaletteMode::PipeOutput => "run with output on stdin",
        _ => "run",
    };
    let hints: Vec<(&[&str], &str)> = if modes_h > 0.0 {
        vec![
            (&["↑", "↓"], "navigate"),
            (&["⏎"], enter),
            (&["⇥"], "next mode"),
        ]
    } else {
        vec![(&["↑", "↓"], "navigate"), (&["⏎"], enter)]
    };
    super::completion::key_hints(
        layer,
        engine,
        theme,
        &hints,
        mx + 16.0,
        fy + FOOTER_H / 2.0,
        mx + modal_w - 16.0,
    );
    layer.set_clip(None);
}

/// Longest prefix of `text` (plus `…`) that fits `max_w`.
pub(crate) fn elide_to(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    family: &'static str,
    max_w: f32,
) -> String {
    if engine.measure(text, size, family, Weight::NORMAL) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // binary search on the prefix length
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if engine.measure(&candidate, size, family, Weight::NORMAL) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// The row icon of an item, from the shared icon set.
fn item_icon(item: &Item) -> Icon {
    match &item.action {
        PaletteAction::Search => Icon::Search,
        PaletteAction::App(AppAction::SplitRight) => Icon::SplitRight,
        PaletteAction::App(AppAction::SplitDown) => Icon::SplitDown,
        PaletteAction::App(AppAction::NewTab) => Icon::Plus,
        PaletteAction::App(AppAction::ClosePane) => Icon::Close,
        PaletteAction::App(AppAction::ToggleExplorer) => Icon::Sidebar,
        PaletteAction::OpenSettings => Icon::Settings,
        PaletteAction::SwitchTheme(_) => Icon::Themes,
        PaletteAction::Launch(_) => Icon::Launch,
        PaletteAction::InsertCommand(_)
        | PaletteAction::RunHistory(_)
        | PaletteAction::OpenHistory => Icon::History,
        PaletteAction::ToggleVim => Icon::Keyboard,
        PaletteAction::FocusTab(_)
        | PaletteAction::FocusPane(..)
        | PaletteAction::FocusWindow(..) => Icon::Terminal,
        PaletteAction::ShowBlock(..) => Icon::ChevronRight,
        PaletteAction::RunPipe => Icon::Filter,
        PaletteAction::Ssh(_) | PaletteAction::SshInstall(_) | PaletteAction::AttachService(_) => {
            Icon::Open
        }
        _ => Icon::ChevronRight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_palette_exposes_full_action_set_and_resolved_shortcuts() {
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("open_settings".into(), "ctrl+,".into());
        let keymap = rift_core::Keymap::from_overrides(&overrides);
        let items = base_items(&["dark-default".into()], &[], &keymap);

        assert!(items.len() >= 42);
        assert!(items.iter().any(|i| i.title == "Command history"));
        assert!(items.iter().any(|i| i.title == "Maximize active pane"));
        assert!(items.iter().any(|i| i.title == "Copy block output"));

        let settings = items
            .iter()
            .find(|i| i.id == "open_settings")
            .expect("settings action");
        assert_eq!(settings.shortcut, vec!["⌃", ","]);
    }

    #[test]
    fn pipe_rows_preview_output_and_never_embed_it() {
        let rows = pipe_items("wc -l", "a\nb\n$(rm -rf ~)\n");
        assert_eq!(rows[0].item.title, "Run `wc -l` with 3 lines on stdin");
        assert_eq!(rows[3].item.title, "$(rm -rf ~)");
        assert!(rows.iter().all(|r| r.item.action == PaletteAction::RunPipe));
        let cmd = pipe_command("grep x | wc -l", "/tmp/rift pipe/1.txt", "/bin/zsh");
        assert_eq!(cmd, "( grep x | wc -l ) < '/tmp/rift pipe/1.txt'");
        let fish = pipe_command("wc -l", "/tmp/p.txt", "/opt/homebrew/bin/fish");
        assert_eq!(fish, "begin; wc -l; end < /tmp/p.txt");
    }
}
