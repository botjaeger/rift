use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

use crate::input::AppAction;
use crate::state::Hit;

/// What running a palette row does.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteAction {
    App(AppAction),
    OpenSettings,
    SwitchTheme(usize),
    RunHistory(String),
    /// ⌃R history search: Enter inserts into the editor instead of running.
    InsertCommand(String),
    /// ⌃⇧R workflows: index into the app's loaded workflow list.
    InsertWorkflow(usize),
    /// Index into the app's loaded launch configurations.
    Launch(usize),
    ToggleVim,
    Search,
    OpenHistory,
    /// Session switcher: activate tab `.0`.
    FocusTab(usize),
    /// Session switcher: focus pane `.1` of tab `.0`.
    FocusPane(usize, shift_pty::PaneId),
    /// Session switcher: show block `.2` of pane `.1` in tab `.0`.
    ShowBlock(usize, shift_pty::PaneId, shift_core::BlockId),
    /// Pipe mode: run the typed command with the block output on stdin.
    RunPipe,
    /// Open a new tab connected to ssh profile `.0`.
    Ssh(usize),
    /// Attention center: go to entry `.0`.
    ShowAttention(usize),
    /// Open a tab reattached to session-service shell `.0`.
    AttachService(u64),
    /// Session switcher: raise window `.0` (pid) at tab `.1`, pane `.2`.
    FocusWindow(u32, usize, u64),
    /// Ask to install remote shell integration on ssh profile `.0`.
    SshInstall(usize),
}

/// Palette rows for ssh profiles: connect, and install integration.
pub fn ssh_items(profiles: &[shift_core::ssh::SshProfile]) -> Vec<Item> {
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
            title: format!("Install Shift shell integration on {}", p.name),
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Settings,
            action: PaletteAction::SshInstall(i),
            mono_title: false,
        });
    }
    out
}

/// What the modal is searching over (Warp parity: ⌘K / ⌃R / ⌃⇧R).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PaletteMode {
    #[default]
    Commands,
    History,
    Workflows,
    /// ⇧⌘O: tabs, panes and recent blocks of this window.
    Sessions,
    /// Block action: the query is a command; the rows preview the output
    /// that will be its stdin.
    PipeOutput,
    /// ⇧⌘A: finished commands, failures, notifications and bells.
    Attention,
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
    keymap: &shift_core::Keymap,
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
            "Workflows",
            "workflows",
            PaletteAction::App(AppAction::Workflows),
        ),
        item(
            "Go to tab, pane or block",
            "session_switcher",
            PaletteAction::App(AppAction::SessionSwitcher),
        ),
        item(
            "Attention center: finished commands and alerts",
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
        item("Quit Shift", "quit", PaletteAction::App(AppAction::Quit)),
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

/// ⌃⇧R mode: loaded workflows.
pub fn workflow_items(workflows: &[shift_core::workflows::Workflow]) -> Vec<Item> {
    workflows
        .iter()
        .enumerate()
        .map(|(i, w)| Item {
            title: w.name.clone(),
            id: w
                .command
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(40)
                .collect(),
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::InsertWorkflow(i),
            mono_title: false,
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

const MODAL_W: f32 = 600.0;
const ROW_H: f32 = 42.0;
const SECTION_H: f32 = 28.0;

fn sans(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
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

/// Render the palette modal over a dimmed backdrop (design 03).
/// Returns click targets: (row_rect, index).
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
    let _ = hits;
    // dim backdrop below the tab bar
    layer.rect(
        [0.0, crate::views::tabbar::TAB_BAR_H],
        [w, h - crate::views::tabbar::TAB_BAR_H],
        Color::hex("#0d0d0f").unwrap().with_alpha(0.66),
        0.0,
    );

    let mx = (w - MODAL_W) / 2.0;
    let my = crate::views::tabbar::TAB_BAR_H + 46.0;

    // Keep the keyboard-selected row visible when the result list is longer
    // than the modal. Row hit indices remain absolute into `results`.
    let max_rows = 8usize;
    let shown = results.len().min(max_rows);
    let start = state
        .selected
        .saturating_add(1)
        .saturating_sub(max_rows)
        .min(results.len().saturating_sub(shown));
    let visible = &results[start..start + shown];
    let mut list_h = 7.0 * 2.0;
    let mut last_section = None;
    for m in visible {
        if last_section != Some(m.item.section) && m.item.section != Section::Actions {
            list_h += SECTION_H;
        }
        last_section = Some(m.item.section);
        list_h += ROW_H;
    }
    let search_h = 49.0;
    let footer_h = 38.0;
    let modal_h = search_h + list_h + footer_h;

    // modal shadow + surface
    layer.shadow(
        [mx, my + 10.0],
        [MODAL_W, modal_h],
        Color::hex("#000000").unwrap().with_alpha(0.55),
        13.0,
        18.0,
    );
    layer.bordered(
        [mx, my],
        [MODAL_W, modal_h],
        theme.surface,
        Color::rgba(1.0, 1.0, 1.0, 0.11),
        13.0,
        1.0,
    );

    // ── search box ──
    let sx = mx + 18.0;
    let sy = my + 15.0;
    // magnifier icon: ring + handle
    layer.bordered(
        [sx, sy + 1.0],
        [12.0, 12.0],
        Color::TRANSPARENT,
        theme.text_dim,
        6.0,
        1.8,
    );
    // diagonal handle (design: 45° stroke) stepped from round dots
    for i in 0..4 {
        let d = i as f32 * 1.2;
        layer.rect(
            [sx + 9.6 + d, sy + 10.6 + d],
            [2.0, 2.0],
            theme.text_dim,
            1.0,
        );
    }

    let qx = sx + 17.0 + 12.0;
    if !state.query.is_empty() {
        let qbuf = mono(engine, &state.query, 15.0, Weight::NORMAL);
        layer.text(qbuf, qx, sy - 1.0, theme.text_bright);
    } else if let Some(placeholder) = match state.mode {
        PaletteMode::Sessions => Some("Go to tab, pane or block…"),
        PaletteMode::Attention => Some("Filter alerts…"),
        PaletteMode::PipeOutput => Some("Command that reads the output on stdin…"),
        _ => None,
    } {
        let pbuf = mono(engine, placeholder, 15.0, Weight::NORMAL);
        layer.text(pbuf, qx + 6.0, sy - 1.0, theme.text_muted);
    }
    // caret
    if blink_on {
        let qw = engine.measure(&state.query, 15.0, mono_family(), Weight::NORMAL);
        layer.rect([qx + qw + 1.0, sy - 2.0], [2.0, 18.0], theme.accent, 0.0);
    }
    // result count
    let count = format!("{} results", results.len());
    let cw = engine.measure(&count, 11.0, FONT_SANS, Weight::NORMAL);
    let cbuf = sans(engine, &count, 11.0, Weight::NORMAL);
    layer.text(cbuf, mx + MODAL_W - cw - 18.0, sy + 2.0, theme.text_muted);

    layer.hline(
        mx,
        my + search_h,
        MODAL_W,
        1.0,
        Color::rgba(1.0, 1.0, 1.0, 0.07),
    );

    // ── results ──
    let mut y = my + search_h + 7.0;
    let mut last_section = None;
    for (visible_i, m) in visible.iter().enumerate() {
        let i = start + visible_i;
        if last_section != Some(m.item.section) && m.item.section != Section::Actions {
            let label = match m.item.section {
                Section::Settings => "SETTINGS",
                Section::Recent => "RECENT",
                Section::Tabs => "TABS",
                Section::Panes => "PANES",
                Section::Blocks => "RECENT BLOCKS",
                Section::Preview => "OUTPUT (STDIN)",
                Section::Actions => "",
            };
            let lbuf = sans(engine, label, 10.5, Weight::SEMIBOLD);
            layer.text(lbuf, mx + 18.0, y + 9.0, theme.text_muted);
            y += SECTION_H;
        }
        last_section = Some(m.item.section);

        let row_rect = [mx + 7.0, y, MODAL_W - 14.0, ROW_H];
        let selected = i == state.selected;
        if selected {
            layer.rect(
                [row_rect[0], row_rect[1]],
                [row_rect[2], row_rect[3]],
                theme.accent.with_alpha(0.14),
                8.0,
            );
            layer.rect(
                [row_rect[0], row_rect[1]],
                [2.0, row_rect[3]],
                theme.accent,
                1.0,
            );
        }
        row_hits.push((row_rect, i));

        // icon box
        let ix = row_rect[0] + 11.0;
        let iy = y + (ROW_H - 24.0) / 2.0;
        let icon_bg = if m.item.section == Section::Actions {
            theme.accent.with_alpha(if selected { 0.16 } else { 0.12 })
        } else {
            Color::rgba(1.0, 1.0, 1.0, 0.05)
        };
        layer.rect([ix, iy], [24.0, 24.0], icon_bg, 6.0);
        let icon_color = if m.item.section == Section::Actions {
            theme.accent
        } else {
            theme.text_dim
        };
        draw_icon(layer, &m.item, [ix + 5.5, iy + 5.5], icon_color);

        // title with match highlighting
        let tx = ix + 24.0 + 13.0;
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
        // everything left of the shortcut chips must fit the row
        let chips_w: f32 = m
            .item
            .shortcut
            .iter()
            .map(|k| engine.measure(k, 11.0, mono_family(), Weight::NORMAL) + 19.0)
            .sum();
        let max_right = mx + MODAL_W - 18.0 - chips_w - 8.0;
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
                        attrs.weight(Weight::BOLD).color(text_color(theme.accent)),
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
            let idbuf = mono(engine, &id, 11.0, Weight::NORMAL);
            layer.text(idbuf, id_x, y + (ROW_H - 14.0) / 2.0, theme.text_muted);
        }

        // shortcut chips (right-aligned)
        let mut chip_x = mx + MODAL_W - 18.0;
        for key in m.item.shortcut.iter().rev() {
            let kw = engine.measure(key, 11.0, mono_family(), Weight::NORMAL);
            let chip_w = kw + 14.0;
            chip_x -= chip_w;
            layer.bordered(
                [chip_x, y + (ROW_H - 20.0) / 2.0],
                [chip_w, 20.0],
                Color::rgba(1.0, 1.0, 1.0, 0.08),
                Color::rgba(1.0, 1.0, 1.0, 0.08),
                5.0,
                1.0,
            );
            let kbuf = mono(engine, key, 11.0, Weight::NORMAL);
            layer.text(kbuf, chip_x + 7.0, y + (ROW_H - 14.0) / 2.0, theme.text_ui);
            chip_x -= 5.0;
        }

        y += ROW_H;
    }

    // ── footer ──
    let fy = my + modal_h - footer_h;
    layer.rect(
        [mx, fy],
        [MODAL_W, footer_h],
        Color::hex("#000000").unwrap().with_alpha(0.18),
        0.0,
    );
    layer.hline(mx, fy, MODAL_W, 1.0, Color::rgba(1.0, 1.0, 1.0, 0.07));
    let mut fx = mx + 18.0;
    let enter = match state.mode {
        PaletteMode::Sessions | PaletteMode::Attention => " go to",
        PaletteMode::PipeOutput => " run with output on stdin",
        _ => " run",
    };
    for (keys, label) in [("↑↓", " navigate"), ("↵", enter), ("esc", " dismiss")] {
        let kbuf = sans(engine, keys, 11.0, Weight::NORMAL);
        let kw = engine.measure(keys, 11.0, FONT_SANS, Weight::NORMAL);
        layer.text(kbuf, fx, fy + 12.0, theme.text_dim);
        fx += kw;
        let lbuf = sans(engine, label, 11.0, Weight::NORMAL);
        let lw = engine.measure(label, 11.0, FONT_SANS, Weight::NORMAL);
        layer.text(lbuf, fx, fy + 12.0, theme.text_muted);
        fx += lw + 18.0;
    }
    let brand = "Shift palette";
    let bw = engine.measure(brand, 11.0, FONT_SANS, Weight::NORMAL);
    let bbuf = sans(engine, brand, 11.0, Weight::NORMAL);
    layer.text(bbuf, mx + MODAL_W - bw - 18.0, fy + 12.0, theme.accent);
}

/// Longest prefix of `text` (plus `…`) that fits `max_w`.
fn elide_to(
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

/// Tiny quad-drawn icons per item kind.
fn draw_icon(layer: &mut Layer, item: &Item, pos: [f32; 2], color: Color) {
    let [x, y] = pos;
    match &item.action {
        PaletteAction::Search => {
            layer.bordered(
                [x + 1.0, y + 1.0],
                [8.0, 8.0],
                Color::TRANSPARENT,
                color,
                4.0,
                1.5,
            );
            for i in 0..3 {
                let d = i as f32 * 1.1;
                layer.rect([x + 7.6 + d, y + 7.6 + d], [1.6, 1.6], color, 0.8);
            }
        }
        PaletteAction::App(AppAction::SplitRight) => {
            layer.bordered(
                [x, y + 1.0],
                [5.5, 11.0],
                Color::TRANSPARENT,
                color,
                1.5,
                1.3,
            );
            layer.bordered(
                [x + 7.5, y + 1.0],
                [5.5, 11.0],
                Color::TRANSPARENT,
                color,
                1.5,
                1.3,
            );
        }
        PaletteAction::App(AppAction::SplitDown) => {
            layer.bordered(
                [x + 1.0, y],
                [11.0, 5.5],
                Color::TRANSPARENT,
                color,
                1.5,
                1.3,
            );
            layer.bordered(
                [x + 1.0, y + 7.5],
                [11.0, 5.5],
                Color::TRANSPARENT,
                color,
                1.5,
                1.3,
            );
        }
        PaletteAction::App(AppAction::NewTab) => {
            layer.rect([x + 5.5, y + 1.0], [2.0, 11.0], color, 1.0);
            layer.rect([x + 1.0, y + 5.5], [11.0, 2.0], color, 1.0);
        }
        PaletteAction::App(AppAction::ClosePane) => {
            layer.bordered(
                [x + 1.0, y + 1.0],
                [11.0, 11.0],
                Color::TRANSPARENT,
                color,
                2.0,
                1.3,
            );
        }
        PaletteAction::OpenSettings | PaletteAction::SwitchTheme(_) => {
            layer.bordered(
                [x + 3.0, y + 3.0],
                [7.0, 7.0],
                Color::TRANSPARENT,
                color,
                3.5,
                1.5,
            );
            layer.rect([x + 6.0, y], [1.4, 3.0], color, 0.7);
            layer.rect([x + 6.0, y + 10.0], [1.4, 3.0], color, 0.7);
            layer.rect([x, y + 6.0], [3.0, 1.4], color, 0.7);
            layer.rect([x + 10.0, y + 6.0], [3.0, 1.4], color, 0.7);
        }
        PaletteAction::InsertWorkflow(_) => {
            layer.bordered(
                [x + 1.0, y + 1.0],
                [11.0, 11.0],
                Color::TRANSPARENT,
                color,
                2.5,
                1.3,
            );
            layer.rect([x + 3.5, y + 6.0], [6.0, 1.4], color, 0.7);
        }
        PaletteAction::InsertCommand(_) | PaletteAction::RunHistory(_) => {
            layer.bordered(
                [x + 1.0, y + 1.0],
                [11.0, 11.0],
                Color::TRANSPARENT,
                color,
                5.5,
                1.4,
            );
            layer.rect([x + 6.0, y + 3.5], [1.4, 3.5], color, 0.7);
            layer.rect([x + 6.0, y + 6.0], [3.0, 1.4], color, 0.7);
        }
        _ => {
            // Generic command glyph for actions without a specialized icon.
            // Every palette row should remain visually identifiable.
            layer.bordered(
                [x + 2.0, y + 2.0],
                [9.0, 9.0],
                Color::TRANSPARENT,
                color,
                4.5,
                1.3,
            );
            layer.rect([x + 5.0, y + 5.0], [3.0, 3.0], color, 1.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_palette_exposes_full_action_set_and_resolved_shortcuts() {
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("open_settings".into(), "ctrl+,".into());
        let keymap = shift_core::Keymap::from_overrides(&overrides);
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
        let cmd = pipe_command("grep x | wc -l", "/tmp/shift pipe/1.txt", "/bin/zsh");
        assert_eq!(cmd, "( grep x | wc -l ) < '/tmp/shift pipe/1.txt'");
        let fish = pipe_command("wc -l", "/tmp/p.txt", "/opt/homebrew/bin/fish");
        assert_eq!(fish, "begin; wc -l; end < /tmp/p.txt");
    }
}
