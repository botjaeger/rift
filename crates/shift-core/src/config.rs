use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

/// Where the command editor sits (Warp "Input position").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InputPosition {
    #[default]
    Bottom,
    Top,
}

/// What renders above the editor: Shift's context chips, or the shell's
/// own prompt (PS1 / starship / p10k) left visible (Warp "Use my shell's
/// prompt").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    #[default]
    #[serde(alias = "runway")]
    Shift,
    Shell,
}

/// Terminal bell handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Bell {
    #[default]
    Visual,
    Audible,
    Both,
    None,
}

/// User configuration — `~/.config/shift/config.toml`, hot-reloaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    /// Multiplier (spec: terminal output 1.65).
    pub line_height: f32,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    /// Theme name: builtin (`dark-default`, `catppuccin-mocha`, `tokyo-night`)
    /// or a file stem in `~/.config/shift/themes/`.
    pub theme: String,
    pub scrollback: usize,
    pub window_padding: f32,
    pub ligatures: bool,
    /// Reopen last session's tabs (with cwds) on launch.
    pub restore_session: bool,
    pub shell: Option<String>,
    /// Theme used while the OS is in light mode when `sync_os_theme` is on.
    pub theme_light: String,
    /// Follow the OS light/dark appearance (`theme` = dark, `theme_light` = light).
    pub sync_os_theme: bool,
    /// Window background opacity, 0.3–1.0.
    pub window_opacity: f32,
    pub input_position: InputPosition,
    pub prompt: PromptMode,
    /// Copy the selection to the clipboard as soon as the mouse is released.
    pub copy_on_select: bool,
    /// Inline ghost-text suggestion after the cursor.
    pub autosuggestions: bool,
    /// Completion dropdown (Tab still accepts the ghost suggestion).
    pub completions_menu: bool,
    /// Automatically insert and remove matching quotes, brackets and braces.
    pub autocomplete_pairs: bool,
    /// Modal (vim) editing in the command editor.
    pub vim_mode: bool,
    /// Desktop notification when a long command finishes while the window is
    /// not focused.
    pub notify_on_completion: bool,
    /// Minimum command duration (seconds) for a completion notification.
    pub notify_threshold_secs: u64,
    pub bell: Bell,
    /// Warn before closing a pane, tab or window (or quitting) while a
    /// command is still running in it.
    pub confirm_close_running: bool,
    /// Where new tabs and panes start: `"previous"` (the active pane's
    /// directory), `"home"`, or an absolute path (`~` expands).
    pub working_directory: String,
    /// Expand a shell alias inline when Space is typed right after it.
    pub alias_expansion: bool,
    /// Underline an unknown command name in the editor (error highlighting).
    pub error_underline: bool,
    /// Suggest a local fix after a failed "command not found".
    pub command_corrections: bool,
    /// System-wide show/hide shortcut (e.g. `"alt+space"`); empty = off.
    pub global_hotkey: String,
    /// macOS: the Option key sends Meta (ESC prefix, for emacs/readline
    /// word motion) instead of composing characters like `∫`. Linux Alt is
    /// always Meta.
    pub option_as_meta: bool,
    /// Report focus changes to apps that ask for them and forward mouse
    /// clicks, drags and the wheel to apps that enable mouse reporting
    /// (vim, tmux, htop). Hold Shift to select text instead.
    pub mouse_reporting: bool,
    /// Overrides: action name → binding (e.g. `new_tab = "ctrl+shift+t"`).
    pub keybindings: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            font_family: "JetBrains Mono".into(),
            font_size: 13.0,
            line_height: 1.65,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            theme: "dark-default".into(),
            scrollback: 10_000,
            window_padding: 8.0,
            ligatures: true,
            restore_session: true,
            shell: None,
            theme_light: "light-default".into(),
            sync_os_theme: false,
            window_opacity: 1.0,
            input_position: InputPosition::Bottom,
            prompt: PromptMode::Shift,
            copy_on_select: false,
            autosuggestions: true,
            completions_menu: true,
            autocomplete_pairs: true,
            vim_mode: false,
            notify_on_completion: true,
            notify_threshold_secs: 10,
            bell: Bell::Visual,
            confirm_close_running: true,
            working_directory: "previous".into(),
            alias_expansion: false,
            error_underline: true,
            command_corrections: true,
            global_hotkey: String::new(),
            option_as_meta: false,
            mouse_reporting: true,
            keybindings: BTreeMap::new(),
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|root| {
            let current = root.join("shift");
            let legacy = root.join("runway");
            if !current.exists() && legacy.exists() {
                legacy
            } else {
                current
            }
        })
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

pub fn themes_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("themes"))
}

impl Config {
    pub fn load() -> Config {
        let Some(path) = config_path() else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(s) => match toml::from_str(&s) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("config.toml parse error, using defaults: {e}");
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = config_path() else {
            return Err(std::io::Error::other("no config dir"));
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, body)
    }

    /// Resolve the configured theme: builtin name or user theme file.
    pub fn resolve_theme(&self) -> crate::Theme {
        crate::theme::load_by_name(&self.theme)
    }

    /// Theme for the current OS appearance: `theme_light` while the OS is
    /// light and `sync_os_theme` is on, else `theme`.
    pub fn resolve_theme_for(&self, os_light: bool) -> crate::Theme {
        if self.sync_os_theme && os_light {
            crate::theme::load_by_name(&self.theme_light)
        } else {
            self.resolve_theme()
        }
    }

    /// Start directory for a new tab or pane. `previous` is the directory of
    /// the pane the user was in; a configured path that does not exist falls
    /// back to home.
    pub fn start_dir(&self, previous: Option<&str>) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let setting = self.working_directory.trim();
        let dir = match setting {
            "" | "previous" => previous.map(PathBuf::from).or(home.clone()),
            "home" => home.clone(),
            path => {
                let expanded = match (path.strip_prefix("~"), &home) {
                    (Some(rest), Some(h)) => h.join(rest.trim_start_matches('/')),
                    _ => PathBuf::from(path),
                };
                Some(expanded)
            }
        };
        dir.filter(|d| d.is_dir()).or(home)
    }

    /// Horizontal window padding (logical px) clamped to 0–40.
    pub fn padding(&self) -> f32 {
        if self.window_padding.is_finite() {
            self.window_padding.clamp(0.0, 40.0)
        } else {
            8.0
        }
    }

    /// Window opacity clamped to the supported range.
    pub fn opacity(&self) -> f32 {
        if self.window_opacity.is_finite() {
            self.window_opacity.clamp(0.3, 1.0)
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut c = Config {
            font_size: 14.0,
            ..Default::default()
        };
        c.keybindings.insert("new_tab".into(), "cmd+t".into());
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let c: Config = toml::from_str("font_size = 15.0").unwrap();
        assert_eq!(c.font_size, 15.0);
        assert_eq!(c.theme, "dark-default");
        assert!(c.ligatures);
    }

    #[test]
    fn enum_fields_parse_lowercase() {
        let c: Config = toml::from_str(
            "input_position = \"top\"\nprompt = \"shell\"\nbell = \"none\"\nvim_mode = true",
        )
        .unwrap();
        assert_eq!(c.input_position, InputPosition::Top);
        assert_eq!(c.prompt, PromptMode::Shell);
        assert_eq!(c.bell, Bell::None);
        assert!(c.vim_mode);
        assert!(toml::from_str::<Config>("input_position = \"middle\"").is_err());
        let legacy: Config = toml::from_str("prompt = \"runway\"").unwrap();
        assert_eq!(legacy.prompt, PromptMode::Shift);
    }

    #[test]
    fn os_theme_sync_picks_light_only_when_enabled() {
        let c = Config {
            theme: "dark-default".into(),
            theme_light: "light-default".into(),
            sync_os_theme: false,
            ..Default::default()
        };
        assert!(!c.resolve_theme_for(true).is_light());
        let c = Config {
            sync_os_theme: true,
            ..c
        };
        assert!(c.resolve_theme_for(true).is_light());
        assert!(!c.resolve_theme_for(false).is_light());
    }

    #[test]
    fn start_dir_modes() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let tmp = std::env::temp_dir();
        let tmp_s = tmp.to_string_lossy().into_owned();
        let mut c = Config::default();
        assert_eq!(c.start_dir(Some(&tmp_s)), Some(tmp.clone()));
        assert_eq!(c.start_dir(None), Some(home.clone()));
        // a vanished previous directory falls back to home
        assert_eq!(
            c.start_dir(Some("/definitely/not/here")),
            Some(home.clone())
        );
        c.working_directory = "home".into();
        assert_eq!(c.start_dir(Some(&tmp_s)), Some(home.clone()));
        c.working_directory = tmp_s.clone();
        assert_eq!(c.start_dir(None), Some(tmp));
        c.working_directory = "~".into();
        assert_eq!(c.start_dir(None), Some(home));
    }

    #[test]
    fn opacity_clamps() {
        let mut c = Config::default();
        assert_eq!(c.opacity(), 1.0);
        c.window_opacity = 0.1;
        assert_eq!(c.opacity(), 0.3);
        c.window_opacity = 7.0;
        assert_eq!(c.opacity(), 1.0);
        c.window_opacity = f32::NAN;
        assert_eq!(c.opacity(), 1.0);
    }
}
