/// Keybinding parsing + lookup. Defaults per spec §4; `[keybindings]`
/// overrides from config.toml win last.
use std::collections::BTreeMap;

/// Named keys a chord may use (what the app's key normalizer produces).
pub const NAMED_KEYS: &[&str] = &[
    "tab",
    "left",
    "right",
    "up",
    "down",
    "pageup",
    "pagedown",
    "home",
    "end",
    "enter",
    "escape",
    "space",
    "backspace",
    "delete",
    "insert",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub cmd: bool,
    /// Lowercase character ("t") or named key ("tab", "left", "pageup", ",").
    pub key: String,
}

impl Binding {
    /// Parse "ctrl+shift+t", "cmd+,", "alt+left", "shift+pageup".
    pub fn parse(s: &str) -> Option<Binding> {
        let mut b = Binding {
            ctrl: false,
            shift: false,
            alt: false,
            cmd: false,
            key: String::new(),
        };
        for part in s.split('+') {
            let p = part.trim().to_lowercase();
            match p.as_str() {
                "ctrl" | "control" | "^" => b.ctrl = true,
                "shift" | "⇧" => b.shift = true,
                "alt" | "opt" | "option" | "meta" => b.alt = true,
                "cmd" | "super" | "command" | "⌘" => b.cmd = true,
                "" => return None,
                _ => {
                    if !b.key.is_empty() {
                        return None; // two non-modifier keys
                    }
                    // "plus" alias: a literal '+' key can't survive split('+')
                    let key = if p == "plus" { "+".into() } else { p };
                    // one character, or a key name input::key_string emits:
                    // anything else could never match and only unbinds
                    if key.chars().count() != 1 && !NAMED_KEYS.contains(&key.as_str()) {
                        return None;
                    }
                    b.key = key;
                }
            }
        }
        if b.key.is_empty() { None } else { Some(b) }
    }

    /// Canonical chord string (`ctrl+alt+shift+cmd+key`) — parses back to
    /// the same binding.
    pub fn chord(&self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.ctrl, "ctrl+"),
            (self.alt, "alt+"),
            (self.shift, "shift+"),
            (self.cmd, "cmd+"),
        ] {
            if on {
                s.push_str(name);
            }
        }
        s.push_str(if self.key == "+" { "plus" } else { &self.key });
        s
    }

    /// Display chips, one per key, for the platform this build runs on.
    pub fn chips(&self) -> Vec<String> {
        self.chips_for(Platform::current())
    }

    /// Display chips, one per key: macOS glyphs (⌃ ⌥ ⇧ ⌘, ⏎ ⌫) on macOS,
    /// words elsewhere (Ctrl, Alt, Shift, Win or Super, Enter, Backspace).
    pub fn chips_for(&self, platform: Platform) -> Vec<String> {
        let mac = platform == Platform::Mac;
        let mut out: Vec<String> = Vec::new();
        for (on, glyph, word) in [
            (self.ctrl, "⌃", "Ctrl"),
            (self.alt, "⌥", "Alt"),
            (self.shift, "⇧", "Shift"),
            (self.cmd, "⌘", platform.super_name()),
        ] {
            if on {
                out.push(if mac { glyph } else { word }.into());
            }
        }
        out.push(match (self.key.as_str(), mac) {
            ("left", _) => "←".into(),
            ("right", _) => "→".into(),
            ("up", _) => "↑".into(),
            ("down", _) => "↓".into(),
            ("tab", _) => "Tab".into(),
            ("pageup", _) => "PgUp".into(),
            ("pagedown", _) => "PgDn".into(),
            ("escape", _) => "Esc".into(),
            ("home", _) => "Home".into(),
            ("end", _) => "End".into(),
            ("enter", true) => "⏎".into(),
            ("backspace", true) => "⌫".into(),
            ("delete", true) => "⌦".into(),
            ("enter", false) => "Enter".into(),
            ("backspace", false) => "Backspace".into(),
            ("delete", false) => "Del".into(),
            ("insert", false) => "Ins".into(),
            ("space", false) => "Space".into(),
            // "Ctrl++" reads as a typo
            ("+", false) => "Plus".into(),
            (k, _) => k.to_uppercase(),
        });
        out
    }

    /// The chord as one string for this platform: `⇧⌘D` on macOS,
    /// `Ctrl+Shift+D` elsewhere.
    pub fn label(&self) -> String {
        self.label_for(Platform::current())
    }

    /// [`Binding::label`] for `platform`.
    pub fn label_for(&self, platform: Platform) -> String {
        let chips = self.chips_for(platform);
        if platform == Platform::Mac {
            chips.concat()
        } else {
            chips.join("+")
        }
    }
}

/// How a fixed chord the app handles itself (not a keymap action) reads on
/// this platform: `hint("ctrl+r")` is `⌃R` on macOS and `Ctrl+R` elsewhere.
pub fn hint(chord: &str) -> String {
    Binding::parse(chord).map(|b| b.label()).unwrap_or_default()
}

/// A gesture's key with this platform's primary modifier (⌘ on macOS, Ctrl
/// elsewhere): `primary_hint("enter")` is `⌘⏎` or `Ctrl+Enter`.
pub fn primary_hint(key: &str) -> String {
    primary_hint_for(key, Platform::current())
}

/// [`primary_hint`] for `platform`.
pub fn primary_hint_for(key: &str, platform: Platform) -> String {
    let chord = if platform == Platform::Mac {
        format!("cmd+{key}")
    } else {
        format!("ctrl+{key}")
    };
    Binding::parse(&chord)
        .map(|b| b.label_for(platform))
        .unwrap_or_default()
}

/// One key cap written as a macOS glyph (`⏎`, `⌃`, `⇥`), as `platform`
/// names it: unchanged on macOS, a word elsewhere. Other caps pass through.
pub fn cap_for(cap: &str, platform: Platform) -> &str {
    if platform == Platform::Mac {
        return cap;
    }
    match cap {
        "⌃" => "Ctrl",
        "⌥" => "Alt",
        "⇧" => "Shift",
        "⌘" => platform.super_name(),
        "⏎" | "↵" => "Enter",
        "⇥" => "Tab",
        "⌫" => "Backspace",
        "⌦" => "Del",
        "esc" => "Esc",
        _ => cap,
    }
}

/// Prose that names keys with macOS glyphs ("⌃R, ⌃T and ⌥C run",
/// "⌥↑ ⌥↓ reorder", "⌘-click"), as `platform` writes them: unchanged on
/// macOS, "Ctrl+R, Ctrl+T and Alt+C run" elsewhere.
pub fn words_for(text: &str, platform: Platform) -> std::borrow::Cow<'_, str> {
    const GLYPHS: &str = "⌃⌥⇧⌘⏎⇥⌫⌦";
    if platform == Platform::Mac || !text.chars().any(|c| GLYPHS.contains(c)) {
        return text.into();
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !GLYPHS.contains(c) {
            out.push(c);
            continue;
        }
        out.push_str(cap_for(c.encode_utf8(&mut [0; 4]), platform));
        // a modifier joins the key after it: ⌃R → Ctrl+R, not ⌘-click
        let modifier = "⌃⌥⇧⌘".contains(c);
        if modifier
            && chars
                .peek()
                .is_some_and(|n| !n.is_whitespace() && !"-,.;:)".contains(*n))
        {
            out.push('+');
        }
    }
    out.into()
}

/// The platform shortcuts are shown for and whose default keys apply.
/// A parameter (not only `cfg!`) so tests cover every platform on any host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    /// The platform this build runs on (the BSDs count as Linux).
    pub const fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }

    /// The modifier gestures use (⌘-click, ⌘⏎): ⌘ on macOS, Ctrl elsewhere.
    pub const fn primary_name(self) -> &'static str {
        match self {
            Platform::Mac => "⌘",
            Platform::Windows | Platform::Linux => "Ctrl",
        }
    }

    /// What the `cmd` modifier (winit's Super) is called on the keyboard.
    pub const fn super_name(self) -> &'static str {
        match self {
            Platform::Mac => "⌘",
            Platform::Windows => "Win",
            Platform::Linux => "Super",
        }
    }
}

/// The action an entry of [`defaults`] binds: `_alt` (a second chord) and
/// `_pc` (the Windows and Linux chord of a ⌘-only action) name their base.
fn base_action(entry: &str) -> &str {
    entry
        .strip_suffix("_alt")
        .or_else(|| entry.strip_suffix("_pc"))
        .unwrap_or(entry)
}

/// Default action → binding table for this build's platform.
pub fn defaults() -> Vec<(&'static str, &'static str)> {
    defaults_for(Platform::current())
}

/// Default action → binding table (spec §4, Warp-parity layout). Every
/// entry applies on every platform, except the `_pc` ones: Windows and Linux
/// only, so a ⌘-only action has a key there while macOS keeps its keys
/// (and the shell keeps every ⌃ chord) exactly as before.
pub fn defaults_for(platform: Platform) -> Vec<(&'static str, &'static str)> {
    let mut all = shared_defaults();
    if platform != Platform::Mac {
        all.extend(pc_defaults());
    }
    all
}

/// The `_pc` defaults: Warp's Windows/Linux keys
/// (docs.warp.dev/getting-started/keyboard-shortcuts) where Warp binds the
/// action and the chord is free here; otherwise Warp's own mapping rule
/// (⌘X → Ctrl+Shift+X, ⇧⌘X → Ctrl+Alt+X when Ctrl+Shift+X is taken,
/// ⌥⌘X → Ctrl+Alt+X), or Windows Terminal / browser conventions.
fn pc_defaults() -> Vec<(&'static str, &'static str)> {
    vec![
        // Warp's Ctrl+Alt+arrows are subword motion here (as in Warp's
        // editor and VS Code): Shift added
        ("navigate_left_pc", "ctrl+alt+shift+left"),
        ("navigate_right_pc", "ctrl+alt+shift+right"),
        ("navigate_up_pc", "ctrl+alt+shift+up"),
        ("navigate_down_pc", "ctrl+alt+shift+down"),
        // Warp has none; Windows Terminal's Alt+Shift+arrows (←/→ only:
        // Alt+Shift+↑/↓ are the bookmark keys), Ctrl+Alt+↑/↓ for the others
        ("resize_pane_left_pc", "alt+shift+left"),
        ("resize_pane_right_pc", "alt+shift+right"),
        ("resize_pane_up_pc", "ctrl+alt+up"),
        ("resize_pane_down_pc", "ctrl+alt+down"),
        ("open_settings_pc", "ctrl+,"),
        ("open_keybindings_pc", "ctrl+shift+,"),
        ("clear_blocks_pc", "ctrl+shift+k"),
        ("tab_select_1_pc", "ctrl+1"),
        ("tab_select_2_pc", "ctrl+2"),
        ("tab_select_3_pc", "ctrl+3"),
        ("tab_select_4_pc", "ctrl+4"),
        ("tab_select_5_pc", "ctrl+5"),
        ("tab_select_6_pc", "ctrl+6"),
        ("tab_select_7_pc", "ctrl+7"),
        ("tab_select_8_pc", "ctrl+8"),
        ("tab_select_9_pc", "ctrl+9"),
        ("reopen_closed_tab_pc", "ctrl+alt+t"),
        ("font_increase_pc", "ctrl+="),
        ("font_decrease_pc", "ctrl+-"),
        ("font_reset_pc", "ctrl+0"),
        ("focus_input_pc", "ctrl+shift+l"),
        ("maximize_pane_pc", "ctrl+shift+enter"),
        ("select_block_prev_pc", "ctrl+up"),
        ("select_block_next_pc", "ctrl+down"),
        // Warp's Ctrl+Shift+C is copy here
        ("copy_block_command_pc", "ctrl+alt+c"),
        ("copy_block_output_pc", "ctrl+alt+shift+c"),
        // Warp's Ctrl+Shift+I is inspect_command here
        ("reinput_pc", "ctrl+alt+i"),
        ("toggle_sync_inputs_pc", "ctrl+alt+shift+i"),
        ("focus_rail_search_pc", "ctrl+alt+f"),
        ("select_all_pc", "ctrl+shift+a"),
        ("new_window_pc", "ctrl+shift+n"),
        ("close_window_pc", "ctrl+alt+w"),
        // browsers' quit; plain Ctrl+Q is the terminal's XON
        ("quit_pc", "ctrl+shift+q"),
        ("toggle_fullscreen_pc", "f11"),
        ("session_switcher_pc", "ctrl+shift+o"),
        ("hint_mode_pc", "ctrl+shift+h"),
        ("attention_center_pc", "ctrl+alt+a"),
        // Ctrl+Shift+E splits; Ctrl+Alt+E is € (AltGr+E) on most European
        // layouts
        ("toggle_explorer_pc", "ctrl+alt+shift+e"),
        ("toggle_review_pc", "ctrl+alt+g"),
        ("focus_review_pc", "ctrl+alt+shift+g"),
    ]
}

/// The defaults every platform shares.
fn shared_defaults() -> Vec<(&'static str, &'static str)> {
    vec![
        ("new_tab", "ctrl+shift+t"),
        ("new_tab_alt", "cmd+t"),
        ("close_pane", "ctrl+shift+w"),
        ("close_pane_alt", "cmd+w"),
        ("split_vertical", "ctrl+shift+d"),
        ("split_vertical_alt", "cmd+d"),
        ("split_horizontal", "ctrl+shift+e"),
        ("split_horizontal_alt", "shift+cmd+d"),
        ("navigate_left", "alt+cmd+left"),
        ("navigate_right", "alt+cmd+right"),
        ("navigate_up", "alt+cmd+up"),
        ("navigate_down", "alt+cmd+down"),
        ("tab_next", "ctrl+tab"),
        ("tab_next_alt", "shift+cmd+}"),
        ("tab_prev", "ctrl+shift+tab"),
        ("tab_prev_alt", "shift+cmd+{"),
        ("copy", "ctrl+shift+c"),
        ("copy_alt", "cmd+c"),
        ("paste", "ctrl+shift+v"),
        ("paste_alt", "cmd+v"),
        ("search", "ctrl+shift+f"),
        ("search_alt", "cmd+f"),
        ("bookmark_toggle", "ctrl+shift+b"),
        ("bookmark_toggle_alt", "cmd+b"),
        ("bookmark_prev", "alt+shift+up"),
        ("bookmark_prev_alt", "cmd+shift+up"),
        ("bookmark_next", "alt+shift+down"),
        ("bookmark_next_alt", "cmd+shift+down"),
        ("bookmark_closest_prev", "alt+up"),
        ("bookmark_closest_next", "alt+down"),
        ("scroll_up", "shift+pageup"),
        ("scroll_down", "shift+pagedown"),
        ("scroll_page_up", "pageup"),
        ("scroll_page_down", "pagedown"),
        ("scroll_top", "home"),
        ("scroll_bottom", "end"),
        ("open_settings", "cmd+,"),
        ("open_keybindings", "ctrl+cmd+k"),
        ("palette", "cmd+p"),
        ("palette_alt", "ctrl+shift+p"),
        ("clear_blocks", "cmd+k"),
        ("tab_select_1", "cmd+1"),
        ("tab_select_2", "cmd+2"),
        ("tab_select_3", "cmd+3"),
        ("tab_select_4", "cmd+4"),
        ("tab_select_5", "cmd+5"),
        ("tab_select_6", "cmd+6"),
        ("tab_select_7", "cmd+7"),
        ("tab_select_8", "cmd+8"),
        ("tab_select_9", "cmd+9"),
        // the + menu's profiles, in its order (Windows Terminal ⌃⇧1…9)
        ("new_tab_profile_1", "ctrl+shift+1"),
        ("new_tab_profile_2", "ctrl+shift+2"),
        ("new_tab_profile_3", "ctrl+shift+3"),
        ("new_tab_profile_4", "ctrl+shift+4"),
        ("new_tab_profile_5", "ctrl+shift+5"),
        ("new_tab_profile_6", "ctrl+shift+6"),
        ("new_tab_profile_7", "ctrl+shift+7"),
        ("new_tab_profile_8", "ctrl+shift+8"),
        ("new_tab_profile_9", "ctrl+shift+9"),
        ("move_tab_left", "ctrl+shift+left"),
        ("move_tab_right", "ctrl+shift+right"),
        ("reopen_closed_tab", "shift+cmd+t"),
        ("font_increase", "cmd+="),
        ("font_increase_alt", "shift+cmd+plus"),
        ("font_decrease", "cmd+-"),
        ("font_reset", "cmd+0"),
        ("focus_input", "cmd+l"),
        ("maximize_pane", "shift+cmd+enter"),
        ("resize_pane_left", "ctrl+cmd+left"),
        ("resize_pane_right", "ctrl+cmd+right"),
        ("resize_pane_up", "ctrl+cmd+up"),
        ("resize_pane_down", "ctrl+cmd+down"),
        ("select_block_prev", "cmd+up"),
        ("select_block_next", "cmd+down"),
        ("copy_block_command", "shift+cmd+c"),
        ("copy_block_output", "alt+shift+cmd+c"),
        ("reinput", "cmd+i"),
        ("inspect_command", "shift+cmd+i"),
        // Warp's multi-cursor keys, the same on every platform
        ("add_cursor_above", "ctrl+shift+up"),
        ("add_cursor_below", "ctrl+shift+down"),
        ("add_next_occurrence", "ctrl+g"),
        ("inspect_command_alt", "ctrl+shift+i"),
        ("block_menu", "ctrl+m"),
        ("filter_block", "alt+shift+f"),
        ("toggle_sync_inputs", "alt+cmd+i"),
        ("focus_rail_search", "shift+cmd+f"),
        ("select_all", "cmd+a"),
        ("new_window", "cmd+n"),
        ("close_window", "shift+cmd+w"),
        ("minimize", "cmd+m"),
        ("hide_app", "cmd+h"),
        ("toggle_fullscreen", "ctrl+cmd+f"),
        ("quit", "cmd+q"),
        ("session_switcher", "shift+cmd+o"),
        ("hint_mode", "shift+cmd+h"),
        ("attention_center", "shift+cmd+a"),
        ("toggle_explorer", "shift+cmd+e"),
        ("toggle_review", "shift+cmd+g"),
        ("focus_review", "alt+cmd+g"),
        // Warp's ⌃G is add_next_occurrence here (and Claude Code's own
        // external-editor key): the composer takes ⌃⇧G / ⌘E
        ("rich_input", "ctrl+shift+g"),
        ("rich_input_alt", "cmd+e"),
        // Windows Terminal's mark mode key (keyboard selection in scrollback)
        ("mark_mode", "ctrl+shift+m"),
        ("mark_mode_alt", "shift+cmd+m"),
        // Warp's editor keys: Meta+. and subword motion
        ("insert_last_argument", "alt+."),
        ("subword_left", "ctrl+alt+left"),
        ("subword_right", "ctrl+alt+right"),
    ]
}

/// Compiled keymap: binding → action name.
#[derive(Debug, Clone)]
pub struct Keymap {
    entries: Vec<(Binding, String)>,
    /// How many entries at the front are `[keybindings]` overrides.
    overrides: usize,
}

impl Keymap {
    /// Defaults, overrides and the `[[actions]]` chords: an action's key
    /// (entry `user_action_<i>`) loses to an override on the same chord and
    /// wins over a default.
    pub fn from_config(
        overrides: &BTreeMap<String, String>,
        actions: &[crate::actions::UserAction],
    ) -> Keymap {
        let mut km = Keymap::from_overrides(overrides);
        let user = actions.iter().enumerate().filter_map(|(i, a)| {
            let b = Binding::parse(a.key.as_deref()?)?;
            Some((b, format!("{}{i}", crate::actions::KEYMAP_PREFIX)))
        });
        let at = km.overrides;
        km.entries.splice(at..at, user);
        km
    }

    /// Defaults + user overrides (overrides win: they are matched first and
    /// also unbind the default for the same action).
    pub fn from_overrides(overrides: &BTreeMap<String, String>) -> Keymap {
        Keymap::from_overrides_for(overrides, Platform::current())
    }

    /// [`Keymap::from_overrides`] with `platform`'s defaults.
    pub fn from_overrides_for(overrides: &BTreeMap<String, String>, platform: Platform) -> Keymap {
        let mut entries: Vec<(Binding, String)> = Vec::new();
        let known = all_actions();
        for (action, binding) in overrides {
            if !known.contains(&action.as_str()) {
                log::warn!("unknown keybinding action {action:?} (ignored)");
                continue;
            }
            // `none` (or empty) unbinds the action's defaults
            if matches!(binding.trim(), "" | "none") {
                continue;
            }
            if let Some(b) = Binding::parse(binding) {
                entries.push((b, action.clone()));
            } else {
                log::warn!("invalid keybinding for {action}: {binding:?}");
            }
        }
        let overrides_len = entries.len();
        for (action, binding) in defaults_for(platform) {
            let base = base_action(action);
            if overrides.contains_key(base) {
                continue; // user override replaces default (and its alt)
            }
            if let Some(b) = Binding::parse(binding) {
                entries.push((b, base.to_string()));
            }
        }
        Keymap {
            entries,
            overrides: overrides_len,
        }
    }

    /// `key` must be pre-normalized: lowercase char or named key string.
    pub fn lookup(&self, ctrl: bool, shift: bool, alt: bool, cmd: bool, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(b, _)| {
                b.ctrl == ctrl && b.shift == shift && b.alt == alt && b.cmd == cmd && b.key == key
            })
            .map(|(_, a)| a.as_str())
    }

    /// Every binding of `action`, in match order (overrides first).
    pub fn bindings_for(&self, action: &str) -> Vec<&Binding> {
        self.entries
            .iter()
            .filter(|(_, a)| a == action)
            .map(|(b, _)| b)
            .collect()
    }

    /// The binding of `action` to show: its first ⌘ one on macOS, its first
    /// one without ⌘ elsewhere (the `_alt` defaults are stored under the
    /// base name, after the Linux-style chord), else its first.
    pub fn preferred(&self, action: &str) -> Option<&Binding> {
        self.preferred_for(action, Platform::current())
    }

    /// [`Keymap::preferred`] as shown on `platform`.
    pub fn preferred_for(&self, action: &str, platform: Platform) -> Option<&Binding> {
        let all = self.bindings_for(action);
        all.iter()
            .find(|b| b.cmd == (platform == Platform::Mac))
            .or(all.first())
            .copied()
    }

    /// For the settings Keybindings page: every bindable action with the
    /// chips of its preferred binding (empty when unbound), sorted by name.
    pub fn display_rows(&self) -> Vec<(String, Vec<String>)> {
        let mut rows: Vec<(String, Vec<String>)> = all_actions()
            .into_iter()
            .map(|a| {
                let chips = self.preferred(a).map(|b| b.chips()).unwrap_or_default();
                (a.to_string(), chips)
            })
            .collect();
        rows.sort();
        rows
    }

    /// The action (other than `except`) already bound to `b`, if any.
    pub fn conflict(&self, b: &Binding, except: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(eb, a)| eb == b && a != except)
            .map(|(_, a)| a.as_str())
    }
}

/// Every action name `[keybindings]` accepts (defaults' base names plus the
/// actions that ship unbound).
pub fn all_actions() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    // every platform's table: a config.toml shared between a Mac and a PC
    // names the same actions
    for (action, _) in defaults_for(Platform::Linux) {
        let base = base_action(action);
        if !out.contains(&base) {
            out.push(base);
        }
    }
    for extra in [
        "rename_tab",
        "close_other_tabs",
        "reset_tab_timer",
        "save_launch_config",
        "equalize_panes",
        "toggle_read_only",
        "restart_pane",
        "pin_tab",
        "toggle_rail_panes",
        "pipe_block_output",
        "new_worktree_tab",
        "toggle_tab_layout",
        "clear_transcripts",
        "clear_ignored_suggestions",
        "whats_new",
        "open_about",
        "check_for_updates",
        "install_cli",
        "update_rift",
        "opacity_increase",
        "opacity_decrease",
        "opacity_reset",
        "toggle_always_on_top",
        "rename_window",
        "theme_picker",
        "cycle_tab_color",
        "set_tab_color",
        "toggle_sticky_header",
        "accept_autosuggestion",
    ] {
        if !out.contains(&extra) {
            out.push(extra);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bindings() {
        let b = Binding::parse("ctrl+shift+t").unwrap();
        assert!(b.ctrl && b.shift && !b.cmd);
        assert_eq!(b.key, "t");
        assert_eq!(Binding::parse("cmd+,").unwrap().key, ",");
        assert!(Binding::parse("ctrl+shift").is_none());
        let plus = Binding::parse("shift+cmd+plus").unwrap();
        assert!(plus.shift && plus.cmd);
        assert_eq!(plus.key, "+");
        // a key name the app never produces could only unbind the action
        assert!(Binding::parse("cmd+foo").is_none());
        assert!(Binding::parse("cmd+f13").is_none());
        assert_eq!(Binding::parse("alt+pagedown").unwrap().key, "pagedown");
        for (action, chord) in defaults() {
            assert!(Binding::parse(chord).is_some(), "{action} = {chord:?}");
        }
    }

    #[test]
    fn unknown_actions_are_ignored() {
        let mut o = BTreeMap::new();
        o.insert("not_an_action".to_string(), "cmd+y".to_string());
        let km = Keymap::from_overrides(&o);
        assert_eq!(km.lookup(false, false, false, true, "y"), None);
    }

    #[test]
    fn defaults_lookup() {
        let km = Keymap::from_overrides(&BTreeMap::new());
        assert_eq!(km.lookup(true, true, false, false, "t"), Some("new_tab"));
        assert_eq!(km.lookup(false, false, false, true, "t"), Some("new_tab"));
        // pane nav moved to alt+cmd (Warp layout); plain alt+left is free
        assert_eq!(
            km.lookup(false, false, true, true, "left"),
            Some("navigate_left")
        );
        assert_eq!(km.lookup(false, false, true, false, "left"), None);
        assert_eq!(km.lookup(false, false, false, false, "t"), None);
    }

    #[test]
    fn warp_layout_defaults() {
        let km = Keymap::from_overrides(&BTreeMap::new());
        assert_eq!(
            km.lookup(false, false, false, true, "k"),
            Some("clear_blocks")
        );
        assert_eq!(km.lookup(false, false, false, true, "p"), Some("palette"));
        assert_eq!(
            km.lookup(false, false, false, true, "1"),
            Some("tab_select_1")
        );
        assert_eq!(km.lookup(false, true, false, true, "}"), Some("tab_next"));
        assert_eq!(
            km.lookup(false, true, false, true, "+"),
            Some("font_increase")
        );
        assert_eq!(
            km.lookup(false, true, false, true, "c"),
            Some("copy_block_command")
        );
        assert_eq!(
            km.lookup(false, true, true, true, "c"),
            Some("copy_block_output")
        );
        assert_eq!(
            km.lookup(false, false, false, false, "pageup"),
            Some("scroll_page_up")
        );
        assert_eq!(
            km.lookup(false, false, false, true, "up"),
            Some("select_block_prev")
        );
        // ctrl+w freed for editor cut-word
        assert_eq!(km.lookup(true, false, false, false, "w"), None);
    }

    #[test]
    fn named_keys_show_as_their_caps() {
        // macOS caps (the words elsewhere: chips_are_glyphs_on_macos_…)
        let chips = |s: &str| Binding::parse(s).unwrap().chips_for(Platform::Mac);
        assert_eq!(chips("cmd+enter"), ["⌘", "⏎"]);
        assert_eq!(chips("escape"), ["Esc"]);
        assert_eq!(chips("alt+backspace"), ["⌥", "⌫"]);
        assert_eq!(chips("delete"), ["⌦"]);
        assert_eq!(chips("home"), ["Home"]);
        assert_eq!(chips("end"), ["End"]);
        assert_eq!(chips("ctrl+k"), ["⌃", "K"]);
    }

    #[test]
    fn the_shown_shortcut_is_the_platform_one() {
        let km = Keymap::from_overrides(&BTreeMap::new());
        // new_tab is ctrl+shift+t, with cmd+t as its `_alt`
        let shown = km.preferred("new_tab").unwrap().chord();
        if cfg!(target_os = "macos") {
            assert_eq!(shown, "cmd+t");
        } else {
            assert_eq!(shown, "ctrl+shift+t");
        }
        let rows = km.display_rows();
        let row = rows.iter().find(|(a, _)| a == "new_tab").unwrap();
        assert_eq!(row.1, km.preferred("new_tab").unwrap().chips());
        // an override replaces both
        let mut o = BTreeMap::new();
        o.insert("new_tab".to_string(), "ctrl+n".to_string());
        let km = Keymap::from_overrides(&o);
        assert_eq!(km.preferred("new_tab").unwrap().chord(), "ctrl+n");
    }

    #[test]
    fn none_unbinds_and_conflicts_are_found() {
        let mut o = BTreeMap::new();
        o.insert("clear_blocks".to_string(), "none".to_string());
        let km = Keymap::from_overrides(&o);
        assert_eq!(km.lookup(false, false, false, true, "k"), None);
        assert!(km.bindings_for("clear_blocks").is_empty());
        let t = Binding::parse("cmd+t").unwrap();
        assert_eq!(km.conflict(&t, "palette"), Some("new_tab"));
        assert_eq!(km.conflict(&t, "new_tab"), None);
        // chords round-trip through the parser
        for s in ["cmd+t", "shift+cmd+plus", "ctrl+alt+shift+cmd+left"] {
            let b = Binding::parse(s).unwrap();
            assert_eq!(Binding::parse(&b.chord()).unwrap(), b);
        }
        let rows = km.display_rows();
        assert!(
            rows.iter()
                .any(|(a, c)| a == "clear_blocks" && c.is_empty())
        );
        assert!(rows.iter().any(|(a, _)| a == "rename_tab"));
    }

    const PLATFORMS: [Platform; 3] = [Platform::Mac, Platform::Windows, Platform::Linux];

    #[test]
    fn no_two_defaults_share_a_chord() {
        for platform in PLATFORMS {
            let mut seen: Vec<(Binding, &str)> = Vec::new();
            for (action, chord) in defaults_for(platform) {
                let b = Binding::parse(chord).unwrap();
                if let Some((_, other)) = seen.iter().find(|(s, _)| *s == b) {
                    panic!("{platform:?}: {chord:?} is both {other} and {action}");
                }
                seen.push((b, action));
            }
        }
    }

    /// ⌘ actions with no key off macOS, and why.
    const MAC_ONLY: &[(&str, &str)] = &[
        ("hide_app", "hiding an app (⌘H) is a macOS concept"),
        (
            "minimize",
            "the system's own key minimizes (Windows ⊞↓, GNOME/KDE Super+H), \
             which ⌘M becomes there",
        ),
    ];

    #[test]
    fn every_cmd_default_has_a_key_off_macos() {
        for platform in [Platform::Windows, Platform::Linux] {
            let km = Keymap::from_overrides_for(&BTreeMap::new(), platform);
            for action in all_actions() {
                let all = km.bindings_for(action);
                if !all.iter().any(|b| b.cmd) {
                    continue;
                }
                let reachable = all.iter().any(|b| !b.cmd);
                let allowed = MAC_ONLY.iter().any(|(a, _)| *a == action);
                assert!(
                    reachable || allowed,
                    "{platform:?}: {action} has only ⌘ defaults; add an `{action}_pc` \
                     chord (or a MAC_ONLY reason)"
                );
                assert!(
                    !(reachable && allowed),
                    "{action} is in MAC_ONLY but has a key"
                );
                // the shown shortcut is the reachable one
                if reachable {
                    assert!(!km.preferred_for(action, platform).unwrap().cmd, "{action}");
                }
            }
        }
    }

    #[test]
    fn pc_defaults_leave_macos_and_the_shell_alone() {
        // macOS: exactly the shared table, no `_pc` entry
        assert_eq!(defaults_for(Platform::Mac), shared_defaults());
        for (action, chord) in pc_defaults() {
            let b = Binding::parse(chord).unwrap();
            assert!(action.ends_with("_pc") && !b.cmd, "{action} = {chord}");
            assert!(
                all_actions().contains(&base_action(action)),
                "{action} names no action"
            );
        }
        // a plain ⌃letter belongs to the shell (⌃A ⌃E ⌃K ⌃R ⌃W …); Warp's
        // two editor keys are the only defaults that take one
        for platform in PLATFORMS {
            for (action, chord) in defaults_for(platform) {
                let b = Binding::parse(chord).unwrap();
                let plain_ctrl_letter = b.ctrl
                    && !b.shift
                    && !b.alt
                    && !b.cmd
                    && b.key.len() == 1
                    && b.key.chars().all(|c| c.is_ascii_alphabetic());
                assert!(
                    !plain_ctrl_letter || ["add_next_occurrence", "block_menu"].contains(&action),
                    "{platform:?}: {action} = {chord} takes a shell key"
                );
            }
        }
    }

    #[test]
    fn chips_are_glyphs_on_macos_and_words_elsewhere() {
        let b = |s: &str| Binding::parse(s).unwrap();
        let d = b("ctrl+shift+d");
        assert_eq!(d.chips_for(Platform::Mac), ["⌃", "⇧", "D"]);
        assert_eq!(d.chips_for(Platform::Windows), ["Ctrl", "Shift", "D"]);
        assert_eq!(d.label_for(Platform::Mac), "⌃⇧D");
        assert_eq!(d.label_for(Platform::Linux), "Ctrl+Shift+D");
        let all = b("ctrl+alt+shift+cmd+enter");
        assert_eq!(all.label_for(Platform::Mac), "⌃⌥⇧⌘⏎");
        assert_eq!(all.label_for(Platform::Windows), "Ctrl+Alt+Shift+Win+Enter");
        assert_eq!(all.label_for(Platform::Linux), "Ctrl+Alt+Shift+Super+Enter");
        for (chord, mac, pc) in [
            ("alt+backspace", "⌥⌫", "Alt+Backspace"),
            ("delete", "⌦", "Del"),
            ("escape", "Esc", "Esc"),
            ("shift+pageup", "⇧PgUp", "Shift+PgUp"),
            ("ctrl+tab", "⌃Tab", "Ctrl+Tab"),
            ("alt+left", "⌥←", "Alt+←"),
            ("ctrl+,", "⌃,", "Ctrl+,"),
            ("ctrl+shift+plus", "⌃⇧+", "Ctrl+Shift+Plus"),
            ("ctrl+space", "⌃SPACE", "Ctrl+Space"),
            ("f11", "F11", "F11"),
        ] {
            assert_eq!(b(chord).label_for(Platform::Mac), mac, "{chord}");
            assert_eq!(b(chord).label_for(Platform::Windows), pc, "{chord}");
            assert_eq!(b(chord).label_for(Platform::Linux), pc, "{chord}");
        }
        // a glyph cap written by hand reads as the chip would: every macOS
        // chip maps through `cap_for` to the same platform's chip
        for platform in [Platform::Windows, Platform::Linux] {
            for chord in [
                "ctrl+alt+shift+cmd+enter",
                "alt+backspace",
                "delete",
                "tab",
                "escape",
            ] {
                let mac = b(chord).chips_for(Platform::Mac);
                let mapped: Vec<&str> = mac.iter().map(|c| cap_for(c, platform)).collect();
                assert_eq!(mapped, b(chord).chips_for(platform), "{platform:?} {chord}");
            }
        }
        assert_eq!(cap_for("⏎", Platform::Mac), "⏎");
        assert_eq!(cap_for("esc", Platform::Mac), "esc");
        assert_eq!(cap_for("esc", Platform::Windows), "Esc");
        assert_eq!(cap_for("⇥", Platform::Linux), "Tab");
        assert_eq!(cap_for("↑", Platform::Linux), "↑");
        // prose
        let pc = |s: &str| words_for(s, Platform::Windows).into_owned();
        assert_eq!(pc("⌃R, ⌃T and ⌥C run"), "Ctrl+R, Ctrl+T and Alt+C run");
        assert_eq!(pc("Drag a row (or ⌥↑ ⌥↓)."), "Drag a row (or Alt+↑ Alt+↓).");
        assert_eq!(pc("With ⇧ it changes"), "With Shift it changes");
        assert_eq!(pc("(⌘-click always opens)"), "(Win-click always opens)");
        assert_eq!(pc("⌃Space completes"), "Ctrl+Space completes");
        assert_eq!(pc("⏎ keeps, esc reverts"), "Enter keeps, esc reverts");
        assert_eq!(
            pc("Esc cancels · ⌫ unbinds"),
            "Esc cancels · Backspace unbinds"
        );
        assert_eq!(words_for("⌘-click", Platform::Linux), "Super-click");
        assert_eq!(words_for("⌃R, ⌥↑", Platform::Mac), "⌃R, ⌥↑");
        // this build's platform
        assert_eq!(d.chips(), d.chips_for(Platform::current()));
        assert_eq!(hint("ctrl+r"), b("ctrl+r").label());
    }

    #[test]
    fn pc_keys_follow_warp_windows_layout() {
        let pc = Keymap::from_overrides_for(&BTreeMap::new(), Platform::Windows);
        let mac = Keymap::from_overrides_for(&BTreeMap::new(), Platform::Mac);
        let at = |km: &Keymap, chord: &str| {
            let b = Binding::parse(chord).unwrap();
            km.lookup(b.ctrl, b.shift, b.alt, b.cmd, &b.key)
                .map(str::to_string)
        };
        for (chord, action) in [
            ("ctrl+,", "open_settings"),
            ("ctrl+shift+k", "clear_blocks"),
            ("ctrl+1", "tab_select_1"),
            ("ctrl+9", "tab_select_9"),
            ("ctrl+alt+t", "reopen_closed_tab"),
            ("ctrl+=", "font_increase"),
            ("ctrl+-", "font_decrease"),
            ("ctrl+0", "font_reset"),
            ("ctrl+shift+l", "focus_input"),
            ("ctrl+shift+enter", "maximize_pane"),
            ("ctrl+up", "select_block_prev"),
            ("ctrl+alt+shift+c", "copy_block_output"),
            ("ctrl+shift+a", "select_all"),
            ("f11", "toggle_fullscreen"),
        ] {
            assert_eq!(at(&pc, chord).as_deref(), Some(action), "{chord}");
            // macOS leaves the chord to the terminal, as before
            assert_eq!(at(&mac, chord), None, "{chord} on macOS");
        }
        // the ⌘ keys still work there (a Win/Super chord), and are not shown
        assert_eq!(at(&pc, "cmd+k").as_deref(), Some("clear_blocks"));
        assert_eq!(
            pc.preferred_for("clear_blocks", Platform::Windows)
                .unwrap()
                .chord(),
            "ctrl+shift+k"
        );
        assert_eq!(
            mac.preferred_for("clear_blocks", Platform::Mac)
                .unwrap()
                .chord(),
            "cmd+k"
        );
        // an override replaces the `_pc` chord too
        let mut o = BTreeMap::new();
        o.insert("clear_blocks".to_string(), "ctrl+alt+l".to_string());
        let km = Keymap::from_overrides_for(&o, Platform::Linux);
        assert_eq!(at(&km, "ctrl+shift+k"), None);
        assert_eq!(km.bindings_for("clear_blocks").len(), 1);
    }

    #[test]
    fn user_action_keys_sit_between_overrides_and_defaults() {
        let action = |name: &str, key: &str| crate::actions::UserAction {
            name: name.into(),
            key: Some(key.into()),
            steps: Vec::new(),
        };
        let mut o = BTreeMap::new();
        o.insert("palette".to_string(), "ctrl+alt+p".to_string());
        let actions = [
            action("over default", "cmd+k"),
            action("under override", "ctrl+alt+p"),
            action("free", "ctrl+alt+h"),
        ];
        let km = Keymap::from_config(&o, &actions);
        // an action's key beats the default ⌘K (clear_blocks)
        assert_eq!(
            km.lookup(false, false, false, true, "k"),
            Some("user_action_0")
        );
        // a [keybindings] entry beats the action's key
        assert_eq!(km.lookup(true, false, true, false, "p"), Some("palette"));
        assert_eq!(
            km.lookup(true, false, true, false, "h"),
            Some("user_action_2")
        );
        // without actions nothing changes
        let plain = Keymap::from_config(&o, &[]);
        assert_eq!(
            plain.lookup(false, false, false, true, "k"),
            Some("clear_blocks")
        );
        assert_eq!(
            Keymap::from_config(&BTreeMap::new(), &[]).lookup(false, true, false, true, "m"),
            Some("mark_mode")
        );
    }

    #[test]
    fn override_wins_and_unbinds_default() {
        let mut o = BTreeMap::new();
        o.insert("new_tab".to_string(), "cmd+n".to_string());
        let km = Keymap::from_overrides(&o);
        assert_eq!(km.lookup(false, false, false, true, "n"), Some("new_tab"));
        assert_eq!(km.lookup(true, true, false, false, "t"), None);
        // override on base name also unbinds the _alt sibling
        assert_eq!(km.lookup(false, false, false, true, "t"), None);
    }
}
