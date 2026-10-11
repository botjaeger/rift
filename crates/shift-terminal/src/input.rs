use winit::event::KeyEvent;
use winit::keyboard::{Key, KeyLocation, ModifiersState, NamedKey};

/// A key press decoupled from winit's `KeyEvent` (which cannot be
/// constructed outside winit) so the script driver can synthesize input
/// through the exact same routing as real keyboard events.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyPress {
    pub logical_key: Key,
    pub text: Option<String>,
    /// The key with modifiers ignored (macOS Option-composed characters map
    /// back to the physical letter) — drives ctrl/meta chords.
    pub unmodified: Option<Key>,
    /// Auto-repeat of a held key.
    pub repeat: bool,
    /// Key release (only forwarded to programs using the kitty keyboard
    /// protocol's event reporting).
    pub released: bool,
    /// Left/right/numpad variant of the key.
    pub location: KeyLocation,
}

impl KeyPress {
    /// A press of `logical_key` producing `text` (standard location, no repeat).
    pub fn new(logical_key: Key, text: Option<String>, unmodified: Option<Key>) -> Self {
        KeyPress {
            logical_key,
            text,
            unmodified,
            repeat: false,
            released: false,
            location: KeyLocation::Standard,
        }
    }
}

/// Stable identity of a physical key across press and release (the key
/// without modifiers, so a Shift change mid-press still matches).
pub fn key_identity(event: &KeyPress) -> String {
    let key = event.unmodified.as_ref().unwrap_or(&event.logical_key);
    let key = match key {
        Key::Character(s) => Key::Character(s.to_lowercase().into()),
        other => other.clone(),
    };
    format!("{key:?}{:?}", event.location)
}

impl From<&KeyEvent> for KeyPress {
    fn from(e: &KeyEvent) -> Self {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        KeyPress {
            logical_key: e.logical_key.clone(),
            text: e.text.as_ref().map(|t| t.to_string()),
            unmodified: Some(e.key_without_modifiers()),
            repeat: e.repeat,
            released: e.state == winit::event::ElementState::Released,
            location: e.location,
        }
    }
}

/// App-level shortcuts (spec §4 default keybindings; user overrides in M7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppAction {
    NewTab,
    ClosePane,
    TabNext,
    TabPrev,
    ScrollUp,
    ScrollDown,
    SplitRight,
    SplitDown,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    TogglePalette,
    OpenSettings,
    Copy,
    Paste,
    Search,
    BookmarkToggle,
    BookmarkPrev,
    BookmarkNext,
    ClearBlocks,
    TabSelect(u8),
    MoveTabLeft,
    MoveTabRight,
    ReopenClosedTab,
    FontIncrease,
    FontDecrease,
    FontReset,
    FocusInput,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    OpenKeybindings,
    /// What's New modal (release notes).
    WhatsNew,
    /// File explorer panel (left).
    ToggleExplorer,
    /// Code review panel (right).
    ToggleReview,
    MaximizePane,
    ResizePaneLeft,
    ResizePaneRight,
    ResizePaneUp,
    ResizePaneDown,
    SelectBlockPrev,
    SelectBlockNext,
    CopyBlockCommand,
    CopyBlockOutput,
    Reinput,
    BlockMenu,
    FilterBlock,
    ToggleSyncInputs,
    SelectAll,
    NewWindow,
    CloseWindow,
    Minimize,
    HideApp,
    ToggleFullscreen,
    Quit,
    RenameTab,
    CloseOtherTabs,
    SaveLaunchConfig,
    /// Searchable switcher over tabs, panes and recent blocks.
    SessionSwitcher,
    /// Letter hints over visible links, paths and block headers.
    HintMode,
    /// Send the selected block's output to a new command's stdin.
    PipeBlockOutput,
    /// Tabs on top ↔ vertical rail.
    ToggleTabLayout,
    /// Delete saved block transcripts.
    ClearTranscripts,
    /// Finished commands, failures, notifications and bells.
    AttentionCenter,
}

impl AppAction {
    /// Actions bound to plain terminal keys (PageUp, Home, ⌃M…) that must
    /// fall through to the PTY when the owned editor is not active.
    /// Whether holding the key keeps firing the action. Everything else
    /// (close, quit, split, new tab, palette ...) runs once per press: an
    /// OS auto-repeat of ⌘W must not close panes down to quitting.
    pub fn repeatable(self) -> bool {
        matches!(
            self,
            AppAction::ScrollUp
                | AppAction::ScrollDown
                | AppAction::ScrollPageUp
                | AppAction::ScrollPageDown
                | AppAction::ScrollTop
                | AppAction::ScrollBottom
                | AppAction::FontIncrease
                | AppAction::FontDecrease
                | AppAction::TabNext
                | AppAction::TabPrev
                | AppAction::FocusLeft
                | AppAction::FocusRight
                | AppAction::FocusUp
                | AppAction::FocusDown
                | AppAction::ResizePaneLeft
                | AppAction::ResizePaneRight
                | AppAction::ResizePaneUp
                | AppAction::ResizePaneDown
                | AppAction::SelectBlockPrev
                | AppAction::SelectBlockNext
                | AppAction::BookmarkPrev
                | AppAction::BookmarkNext
        )
    }

    pub fn needs_editor(self) -> bool {
        matches!(
            self,
            AppAction::ScrollPageUp
                | AppAction::ScrollPageDown
                | AppAction::ScrollTop
                | AppAction::ScrollBottom
                | AppAction::BlockMenu
        )
    }
}

/// Normalize a winit key for keymap lookup: lowercase char or named key.
pub fn key_string(event: &KeyPress) -> Option<String> {
    match &event.logical_key {
        Key::Named(named) => {
            let s = match named {
                NamedKey::Tab => "tab",
                NamedKey::ArrowLeft => "left",
                NamedKey::ArrowRight => "right",
                NamedKey::ArrowUp => "up",
                NamedKey::ArrowDown => "down",
                NamedKey::PageUp => "pageup",
                NamedKey::PageDown => "pagedown",
                NamedKey::Home => "home",
                NamedKey::End => "end",
                NamedKey::Enter => "enter",
                NamedKey::Escape => "escape",
                NamedKey::Space => "space",
                NamedKey::Backspace => "backspace",
                NamedKey::Delete => "delete",
                NamedKey::Insert => "insert",
                NamedKey::F1 => "f1",
                NamedKey::F2 => "f2",
                NamedKey::F3 => "f3",
                NamedKey::F4 => "f4",
                NamedKey::F5 => "f5",
                NamedKey::F6 => "f6",
                NamedKey::F7 => "f7",
                NamedKey::F8 => "f8",
                NamedKey::F9 => "f9",
                NamedKey::F10 => "f10",
                NamedKey::F11 => "f11",
                NamedKey::F12 => "f12",
                _ => return None,
            };
            Some(s.to_string())
        }
        Key::Character(s) => Some(s.to_lowercase()),
        _ => None,
    }
}

pub fn action_from_name(name: &str) -> Option<AppAction> {
    if let Some(n) = name.strip_prefix("tab_select_") {
        return n
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=9).contains(n))
            .map(AppAction::TabSelect);
    }
    Some(match name {
        "new_tab" => AppAction::NewTab,
        "close_pane" => AppAction::ClosePane,
        "split_vertical" => AppAction::SplitRight,
        "split_horizontal" => AppAction::SplitDown,
        "navigate_left" => AppAction::FocusLeft,
        "navigate_right" => AppAction::FocusRight,
        "navigate_up" => AppAction::FocusUp,
        "navigate_down" => AppAction::FocusDown,
        "tab_next" => AppAction::TabNext,
        "tab_prev" => AppAction::TabPrev,
        "scroll_up" => AppAction::ScrollUp,
        "scroll_down" => AppAction::ScrollDown,
        "open_settings" => AppAction::OpenSettings,
        "palette" => AppAction::TogglePalette,
        "copy" => AppAction::Copy,
        "paste" => AppAction::Paste,
        "search" => AppAction::Search,
        "bookmark_toggle" => AppAction::BookmarkToggle,
        "bookmark_prev" | "bookmark_closest_prev" => AppAction::BookmarkPrev,
        "bookmark_next" | "bookmark_closest_next" => AppAction::BookmarkNext,
        "clear_blocks" => AppAction::ClearBlocks,
        "move_tab_left" => AppAction::MoveTabLeft,
        "move_tab_right" => AppAction::MoveTabRight,
        "reopen_closed_tab" => AppAction::ReopenClosedTab,
        "font_increase" => AppAction::FontIncrease,
        "font_decrease" => AppAction::FontDecrease,
        "font_reset" => AppAction::FontReset,
        "focus_input" => AppAction::FocusInput,
        "scroll_page_up" => AppAction::ScrollPageUp,
        "scroll_page_down" => AppAction::ScrollPageDown,
        "scroll_top" => AppAction::ScrollTop,
        "scroll_bottom" => AppAction::ScrollBottom,
        "open_keybindings" => AppAction::OpenKeybindings,
        "whats_new" => AppAction::WhatsNew,
        "toggle_explorer" => AppAction::ToggleExplorer,
        "toggle_review" => AppAction::ToggleReview,
        "maximize_pane" => AppAction::MaximizePane,
        "resize_pane_left" => AppAction::ResizePaneLeft,
        "resize_pane_right" => AppAction::ResizePaneRight,
        "resize_pane_up" => AppAction::ResizePaneUp,
        "resize_pane_down" => AppAction::ResizePaneDown,
        "select_block_prev" => AppAction::SelectBlockPrev,
        "select_block_next" => AppAction::SelectBlockNext,
        "copy_block_command" => AppAction::CopyBlockCommand,
        "copy_block_output" => AppAction::CopyBlockOutput,
        "reinput" => AppAction::Reinput,
        "block_menu" => AppAction::BlockMenu,
        "filter_block" => AppAction::FilterBlock,
        "toggle_sync_inputs" => AppAction::ToggleSyncInputs,
        "select_all" => AppAction::SelectAll,
        "new_window" => AppAction::NewWindow,
        "close_window" => AppAction::CloseWindow,
        "minimize" => AppAction::Minimize,
        "hide_app" => AppAction::HideApp,
        "toggle_fullscreen" => AppAction::ToggleFullscreen,
        "quit" => AppAction::Quit,
        "rename_tab" => AppAction::RenameTab,
        "close_other_tabs" => AppAction::CloseOtherTabs,
        "save_launch_config" => AppAction::SaveLaunchConfig,
        "session_switcher" => AppAction::SessionSwitcher,
        "hint_mode" => AppAction::HintMode,
        "pipe_block_output" => AppAction::PipeBlockOutput,
        "toggle_tab_layout" => AppAction::ToggleTabLayout,
        "clear_transcripts" => AppAction::ClearTranscripts,
        "attention_center" => AppAction::AttentionCenter,
        _ => return None,
    })
}

/// Resolve a key press through the (config-driven) keymap.
pub fn app_action(
    event: &KeyPress,
    mods: ModifiersState,
    keymap: &shift_core::Keymap,
) -> Option<AppAction> {
    // macOS Option composes characters (⌥⇧F = "Ï"); bindings name the
    // physical key, so resolve Option chords through the unmodified key
    let key = match (&event.unmodified, &event.logical_key) {
        (Some(Key::Character(s)), Key::Character(_)) if mods.alt_key() => s.to_lowercase(),
        _ => key_string(event)?,
    };
    let name = keymap.lookup(
        mods.control_key(),
        mods.shift_key(),
        mods.alt_key(),
        mods.super_key(),
        &key,
    )?;
    action_from_name(name)
}

/// Keys consumed by the owned input editor.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorKey {
    Insert(String),
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    DeleteWordBack,
    DeleteWordForward,
    KillToEnd,
    KillToStart,
    WordLeft,
    WordRight,
    ClearLine,
    HistoryUp,
    HistoryDown,
    Submit,
    Interrupt,            // ⌃C
    Eof,                  // ⌃D on empty line
    Complete,             // ⇥
    NextCompletionSource, // ⇧⇥ (completion panel source tabs)
    Dismiss,              // esc
    HistorySearch,        // ⌃R (editor mode only — passthrough keeps raw 0x12)
    AcceptGhostWord,      // ⌃→
    ClearScreen,          // ⌃L (passes \x0c to the shell)
    Undo,                 // ⌘Z
    Redo,                 // ⇧⌘Z
    Cut,                  // ⌘X (editor selection)
    SelectLeft,           // ⇧←
    SelectRight,          // ⇧→
    SelectWordLeft,       // ⌥⇧←
    SelectWordRight,      // ⌥⇧→
    SelectHome,           // ⇧⌘← / ⇧Home
    SelectEnd,            // ⇧⌘→ / ⇧End
}

pub fn editor_key(event: &KeyPress, mods: ModifiersState) -> Option<EditorKey> {
    let ctrl = mods.control_key();
    let alt = mods.alt_key();
    let cmd = mods.super_key();

    if let Key::Named(named) = &event.logical_key
        && mods.shift_key()
        && !ctrl
    {
        let key = match named {
            NamedKey::ArrowLeft if cmd => Some(EditorKey::SelectHome),
            NamedKey::ArrowLeft if alt => Some(EditorKey::SelectWordLeft),
            NamedKey::ArrowLeft => Some(EditorKey::SelectLeft),
            NamedKey::ArrowRight if cmd => Some(EditorKey::SelectEnd),
            NamedKey::ArrowRight if alt => Some(EditorKey::SelectWordRight),
            NamedKey::ArrowRight => Some(EditorKey::SelectRight),
            NamedKey::Home => Some(EditorKey::SelectHome),
            NamedKey::End => Some(EditorKey::SelectEnd),
            _ => None,
        };
        if key.is_some() {
            return key;
        }
    }

    if let Key::Named(named) = &event.logical_key {
        let key = match named {
            // ⇧⏎ / ⌥⏎: newline inside the command (multi-line editing)
            NamedKey::Enter if mods.shift_key() || alt => Some(EditorKey::Insert("\n".into())),
            NamedKey::Enter => Some(EditorKey::Submit),
            NamedKey::Tab if mods.shift_key() => Some(EditorKey::NextCompletionSource),
            NamedKey::Tab => Some(EditorKey::Complete),
            NamedKey::Escape => Some(EditorKey::Dismiss),
            NamedKey::Backspace if cmd => Some(EditorKey::KillToStart),
            NamedKey::Backspace if alt => Some(EditorKey::DeleteWordBack),
            NamedKey::Backspace => Some(EditorKey::Backspace),
            NamedKey::Delete if cmd => Some(EditorKey::KillToEnd),
            NamedKey::Delete if alt => Some(EditorKey::DeleteWordForward),
            NamedKey::Delete => Some(EditorKey::Delete),
            NamedKey::ArrowLeft if cmd => Some(EditorKey::Home),
            NamedKey::ArrowLeft if alt => Some(EditorKey::WordLeft),
            NamedKey::ArrowLeft => Some(EditorKey::Left),
            NamedKey::ArrowRight if cmd => Some(EditorKey::End),
            NamedKey::ArrowRight if ctrl => Some(EditorKey::AcceptGhostWord),
            NamedKey::ArrowRight if alt => Some(EditorKey::WordRight),
            NamedKey::ArrowRight => Some(EditorKey::Right),
            NamedKey::ArrowUp => Some(EditorKey::HistoryUp),
            NamedKey::ArrowDown => Some(EditorKey::HistoryDown),
            NamedKey::Home => Some(EditorKey::Home),
            NamedKey::End => Some(EditorKey::End),
            NamedKey::Space if !ctrl => Some(EditorKey::Insert(" ".into())),
            _ => None,
        };
        if key.is_some() {
            return key;
        }
    }

    // cmd chords never insert text; unbound ones are swallowed (not typed,
    // not leaked to the PTY — see key_to_bytes)
    if cmd {
        if let Key::Character(s) = &event.logical_key {
            match s.to_ascii_lowercase().as_str() {
                "z" if mods.shift_key() => return Some(EditorKey::Redo),
                "z" => return Some(EditorKey::Undo),
                "x" => return Some(EditorKey::Cut),
                _ => {}
            }
        }
        return None;
    }

    if ctrl {
        if let Key::Character(s) = &event.logical_key {
            return match s.to_ascii_lowercase().as_str() {
                "c" => Some(EditorKey::Interrupt),
                "r" => Some(EditorKey::HistorySearch),
                "d" => Some(EditorKey::Eof),
                "u" => Some(EditorKey::ClearLine),
                "a" => Some(EditorKey::Home),
                "e" => Some(EditorKey::End),
                "b" => Some(EditorKey::Left),
                "f" => Some(EditorKey::Right),
                "k" => Some(EditorKey::KillToEnd),
                "w" => Some(EditorKey::DeleteWordBack),
                "l" => Some(EditorKey::ClearScreen),
                _ => None,
            };
        }
        return None;
    }

    if let Some(text) = &event.text
        && !text.is_empty()
        && !text.chars().any(|c| c.is_control())
    {
        return Some(EditorKey::Insert(text.to_string()));
    }
    None
}

/// Terminal-input options that change the bytes a key produces.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KeyEncoding {
    /// DECCKM application cursor mode (`\x1bOA` instead of `\x1b[A`).
    pub app_cursor: bool,
    /// macOS Option acts as Meta (ESC prefix) instead of composing
    /// characters (Warp "Left Option key is Meta").
    pub option_as_meta: bool,
    /// Active kitty keyboard protocol flags (0 = legacy xterm encoding).
    pub kitty_flags: u8,
}

/// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
fn modifier_param(mods: ModifiersState) -> u8 {
    1 + u8::from(mods.shift_key()) + 2 * u8::from(mods.alt_key()) + 4 * u8::from(mods.control_key())
}

/// Bytes for a cursor/function key: `\x1b[1;{m}X` / `\x1b[N;{m}~` when a
/// modifier is held, the plain (or SS3) form otherwise.
fn special_key(named: &NamedKey, m: u8, app_cursor: bool) -> Option<Vec<u8>> {
    let letter = |c: char, ss3: bool| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[1;{m}{c}").into_bytes()
        } else if ss3 {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[{n};{m}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    Some(match named {
        NamedKey::ArrowUp => letter('A', app_cursor),
        NamedKey::ArrowDown => letter('B', app_cursor),
        NamedKey::ArrowRight => letter('C', app_cursor),
        NamedKey::ArrowLeft => letter('D', app_cursor),
        NamedKey::Home => letter('H', app_cursor),
        NamedKey::End => letter('F', app_cursor),
        NamedKey::F1 => letter('P', true),
        NamedKey::F2 => letter('Q', true),
        NamedKey::F3 => letter('R', true),
        NamedKey::F4 => letter('S', true),
        NamedKey::Insert => tilde(2),
        NamedKey::Delete => tilde(3),
        NamedKey::PageUp => tilde(5),
        NamedKey::PageDown => tilde(6),
        NamedKey::F5 => tilde(15),
        NamedKey::F6 => tilde(17),
        NamedKey::F7 => tilde(18),
        NamedKey::F8 => tilde(19),
        NamedKey::F9 => tilde(20),
        NamedKey::F10 => tilde(21),
        NamedKey::F11 => tilde(23),
        NamedKey::F12 => tilde(24),
        _ => return None,
    })
}

/// `c` with Shift held on a US layout (letters upper-cased, digits and
/// punctuation mapped to their shifted symbol).
fn shifted_char(c: char) -> char {
    match c {
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        c => c.to_ascii_uppercase(),
    }
}

/// Control code for ctrl+`c` (`None` when the key has none).
fn control_byte(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | '2' | ' ' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '7' | '/' | '-' => Some(0x1f),
        '8' => Some(0x7f),
        _ => None,
    }
}

/// Translate a key press into the byte sequence written to the PTY.
/// Returns None when the key is not terminal input (handled elsewhere).
pub fn key_to_bytes(event: &KeyPress, mods: ModifiersState, enc: KeyEncoding) -> Option<Vec<u8>> {
    // ⌘⌫ deletes to the start of the line, as in Warp and iTerm2: the
    // program gets ctrl+u, the line editors' kill-to-start key (readline,
    // zle, Claude Code), encoded for its keyboard mode
    if mods == ModifiersState::SUPER && event.logical_key == Key::Named(NamedKey::Backspace) {
        let u = || Key::Character("u".into());
        let ctrl_u = KeyPress {
            repeat: event.repeat,
            released: event.released,
            ..KeyPress::new(u(), None, Some(u()))
        };
        return key_to_bytes(&ctrl_u, ModifiersState::CONTROL, enc);
    }
    // other cmd chords are app territory — never leak them to the PTY
    if mods.super_key() {
        return None;
    }
    if enc.kitty_flags != 0 {
        return crate::keyboard::encode(event, mods, enc);
    }
    if event.released {
        return None;
    }
    let ctrl = mods.control_key();
    let alt = mods.alt_key();
    let shift = mods.shift_key();
    // Option composes characters on macOS unless it is configured as Meta
    let meta = alt && (enc.option_as_meta || !cfg!(target_os = "macos"));
    let esc = |mut b: Vec<u8>| {
        if meta {
            b.insert(0, 0x1b);
        }
        b
    };

    if let Key::Named(named) = &event.logical_key {
        if let Some(bytes) = special_key(named, modifier_param(mods), enc.app_cursor) {
            return Some(bytes);
        }
        let bytes = match named {
            NamedKey::Enter => Some(esc(b"\r".to_vec())),
            NamedKey::Backspace if ctrl => Some(esc(vec![0x08])),
            NamedKey::Backspace => Some(esc(vec![0x7f])),
            NamedKey::Tab if shift => Some(b"\x1b[Z".to_vec()),
            NamedKey::Tab => Some(esc(b"\t".to_vec())),
            NamedKey::Escape => Some(esc(vec![0x1b])),
            NamedKey::Space if ctrl => Some(esc(vec![0])),
            NamedKey::Space => Some(esc(b" ".to_vec())),
            _ => None,
        };
        if bytes.is_some() {
            return bytes;
        }
    }

    // the key as printed without modifiers (Option-composed characters on
    // macOS resolve back to the physical letter for ctrl/meta chords)
    let single = |s: &str| {
        let mut cs = s.chars();
        match (cs.next(), cs.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    };
    let base = match (&event.unmodified, &event.logical_key) {
        (Some(Key::Character(s)), _) | (None, Key::Character(s)) => single(s),
        _ => None,
    };

    // ctrl-<key> control codes
    if ctrl {
        let c = match &event.logical_key {
            Key::Character(s) => single(s).filter(|c| control_byte(*c).is_some()),
            _ => None,
        }
        .or(base);
        return c.and_then(control_byte).map(|b| esc(vec![b]));
    }

    // Meta+key: ESC + the unmodified character (shifted when shift is held)
    if meta && let Some(c) = base {
        let c = if shift { shifted_char(c) } else { c };
        let mut bytes = vec![0x1b];
        let mut tmp = [0u8; 4];
        bytes.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
        return Some(bytes);
    }

    // plain text (IME-composed or direct)
    match &event.text {
        Some(text) if !text.is_empty() => Some(text.as_bytes().to_vec()),
        _ => None,
    }
}

/// Bracketed paste (DECSET 2004): the text between the markers, with any
/// embedded end marker removed so it cannot terminate the paste early.
pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    // Strip to a fixpoint: removing one marker can splice a new one together.
    let mut clean = text.to_string();
    while clean.contains("\x1b[201~") {
        clean = clean.replace("\x1b[201~", "");
    }
    bytes.extend_from_slice(clean.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

/// Text for the owned command editor: line breaks become `\n`, tabs stay,
/// and every other control character (ESC, ^C, ^U, DEL, C1 ...) is dropped so
/// a paste cannot smuggle keystrokes into the submitted command.
pub fn sanitize_editor_paste(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\r' => Some('\n'),
            '\n' | '\t' => Some(c),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// Unbracketed paste: newlines become Enter (CR), like typing them would.
pub fn unbracketed_paste(text: &str) -> Vec<u8> {
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

/// Paste bytes for one recipient, framed for its own paste mode.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        bracketed_paste(text)
    } else {
        unbracketed_paste(text)
    }
}

/// Whether an unbracketed paste of `text` would press Enter.
pub fn paste_submits_lines(text: &str) -> bool {
    text.contains(['\n', '\r'])
}

/// Quote `s` for a POSIX shell when it contains anything but safe path
/// characters (dropped files, pasted paths).
pub fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+,:@%~=".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quoting() {
        assert_eq!(shell_quote("/tmp/a.txt"), "/tmp/a.txt");
        assert_eq!(shell_quote("/tmp/my file"), "'/tmp/my file'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn paste_encoding() {
        assert_eq!(
            bracketed_paste("a\x1b[201~b\n"),
            b"\x1b[200~ab\n\x1b[201~".to_vec()
        );
        // a marker spliced together by the removal of another is stripped too
        assert_eq!(
            bracketed_paste("\x1b[2\x1b[201~01~"),
            b"\x1b[200~\x1b[201~".to_vec()
        );
        assert_eq!(
            sanitize_editor_paste("a\x15b\x1b[31m\x03\r\nc\rd\te\x7f"),
            "ab[31m\nc\nd\te"
        );
        assert_eq!(unbracketed_paste("a\r\nb\nc"), b"a\rb\rc".to_vec());
        assert!(paste_submits_lines("ls\n"));
        assert!(paste_submits_lines("a\rb"));
        assert!(!paste_submits_lines("echo one line"));
    }

    fn named(k: NamedKey) -> KeyPress {
        KeyPress::new(Key::Named(k), None, None)
    }

    fn ch(c: &str, text: Option<&str>, unmodified: &str) -> KeyPress {
        KeyPress::new(
            Key::Character(c.into()),
            text.map(str::to_string),
            Some(Key::Character(unmodified.into())),
        )
    }

    const PLAIN: KeyEncoding = KeyEncoding {
        app_cursor: false,
        option_as_meta: false,
        kitty_flags: 0,
    };
    const NONE: ModifiersState = ModifiersState::empty();

    #[test]
    fn cmd_backspace_kills_to_line_start() {
        let bs = named(NamedKey::Backspace);
        assert_eq!(
            key_to_bytes(&bs, ModifiersState::SUPER, PLAIN),
            Some(vec![0x15])
        );
        // kitty keyboard mode: ctrl+u in CSI u form
        let kitty = KeyEncoding {
            kitty_flags: 1,
            ..PLAIN
        };
        assert_eq!(
            key_to_bytes(&bs, ModifiersState::SUPER, kitty),
            Some(b"\x1b[117;5u".to_vec())
        );
        // other cmd chords stay with the app
        let shifted = ModifiersState::SUPER | ModifiersState::SHIFT;
        assert_eq!(key_to_bytes(&bs, shifted, PLAIN), None);
        assert_eq!(
            key_to_bytes(&ch("k", Some("k"), "k"), ModifiersState::SUPER, PLAIN),
            None
        );
    }

    #[test]
    fn arrows_follow_app_cursor_mode() {
        let up = named(NamedKey::ArrowUp);
        assert_eq!(key_to_bytes(&up, NONE, PLAIN).unwrap(), b"\x1b[A");
        let app = KeyEncoding {
            app_cursor: true,
            ..PLAIN
        };
        assert_eq!(key_to_bytes(&up, NONE, app).unwrap(), b"\x1bOA");
        assert_eq!(
            key_to_bytes(&named(NamedKey::Home), NONE, app).unwrap(),
            b"\x1bOH"
        );
        // modified arrows always use the CSI form
        assert_eq!(
            key_to_bytes(&up, ModifiersState::CONTROL, app).unwrap(),
            b"\x1b[1;5A"
        );
        assert_eq!(
            key_to_bytes(&named(NamedKey::ArrowLeft), ModifiersState::SHIFT, PLAIN).unwrap(),
            b"\x1b[1;2D"
        );
    }

    #[test]
    fn function_keys() {
        let key = |k| key_to_bytes(&named(k), NONE, PLAIN).unwrap();
        assert_eq!(key(NamedKey::F1), b"\x1bOP");
        assert_eq!(key(NamedKey::F4), b"\x1bOS");
        assert_eq!(key(NamedKey::F5), b"\x1b[15~");
        assert_eq!(key(NamedKey::F10), b"\x1b[21~");
        assert_eq!(key(NamedKey::F12), b"\x1b[24~");
        assert_eq!(
            key_to_bytes(&named(NamedKey::F2), ModifiersState::SHIFT, PLAIN).unwrap(),
            b"\x1b[1;2Q"
        );
        assert_eq!(
            key_to_bytes(&named(NamedKey::Delete), ModifiersState::CONTROL, PLAIN).unwrap(),
            b"\x1b[3;5~"
        );
    }

    #[test]
    fn control_codes() {
        let c = ch("c", None, "c");
        assert_eq!(
            key_to_bytes(&c, ModifiersState::CONTROL, PLAIN).unwrap(),
            [3]
        );
        let bs = named(NamedKey::Backspace);
        assert_eq!(
            key_to_bytes(&bs, ModifiersState::CONTROL, PLAIN).unwrap(),
            [8]
        );
        assert_eq!(key_to_bytes(&bs, NONE, PLAIN).unwrap(), [0x7f]);
        assert_eq!(
            key_to_bytes(&named(NamedKey::Space), ModifiersState::CONTROL, PLAIN).unwrap(),
            [0]
        );
    }

    #[test]
    fn ctrl_minus_and_meta_shift_symbols() {
        let minus = ch("-", Some("-"), "-");
        assert_eq!(
            key_to_bytes(&minus, ModifiersState::CONTROL, PLAIN).unwrap(),
            [0x1f]
        );
        let one = ch("!", Some("!"), "1");
        let meta = KeyEncoding {
            option_as_meta: true,
            ..PLAIN
        };
        assert_eq!(
            key_to_bytes(&one, ModifiersState::ALT | ModifiersState::SHIFT, meta).unwrap(),
            b"\x1b!"
        );
        // ctrl+space must not type a space into the editor
        assert_eq!(
            editor_key(&named(NamedKey::Space), ModifiersState::CONTROL),
            None
        );
        assert_eq!(
            editor_key(&named(NamedKey::Space), NONE),
            Some(EditorKey::Insert(" ".into()))
        );
    }

    #[test]
    fn option_as_meta_sends_escape_prefix() {
        // macOS Option+b composes "∫"; as Meta it must send ESC b
        let press = ch("∫", Some("∫"), "b");
        let meta = KeyEncoding {
            option_as_meta: true,
            ..PLAIN
        };
        assert_eq!(
            key_to_bytes(&press, ModifiersState::ALT, meta).unwrap(),
            b"\x1bb"
        );
        assert_eq!(
            key_to_bytes(&named(NamedKey::Backspace), ModifiersState::ALT, meta).unwrap(),
            b"\x1b\x7f"
        );
        if cfg!(target_os = "macos") {
            // composing Option keeps the composed character
            assert_eq!(
                key_to_bytes(&press, ModifiersState::ALT, PLAIN).unwrap(),
                "∫".as_bytes()
            );
        }
    }

    #[test]
    fn held_shortcuts_do_not_repeat_destructive_actions() {
        assert!(!AppAction::ClosePane.repeatable());
        assert!(!AppAction::Quit.repeatable());
        assert!(!AppAction::NewTab.repeatable());
        assert!(AppAction::ScrollUp.repeatable());
        assert!(AppAction::FontIncrease.repeatable());
    }

    #[test]
    fn plain_text_and_cmd() {
        let a = ch("a", Some("a"), "a");
        assert_eq!(key_to_bytes(&a, NONE, PLAIN).unwrap(), b"a");
        assert!(key_to_bytes(&a, ModifiersState::SUPER, PLAIN).is_none());
    }
}
