use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

/// Where the tabs sit: the top strip, or a vertical rail on the left that
/// also shows each tab's directory and git branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TabLayout {
    #[default]
    Horizontal,
    Vertical,
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
    /// Also save finished blocks (command, status, output) and show them
    /// again on relaunch. Output may contain secrets, so this is opt-in.
    pub save_transcripts: bool,
    /// Shells run in the per-user session service, so closing or quitting
    /// Shift leaves them running and relaunching reattaches them.
    pub durable_sessions: bool,
    /// Local control socket: "off", "navigate" (list/status/focus) or
    /// "full" (also split/open/send-text). See control.rs.
    pub control_api: String,
    pub shell: Option<String>,
    /// Theme used while the OS is in light mode when `sync_os_theme` is on.
    pub theme_light: String,
    /// Follow the OS light/dark appearance (`theme` = dark, `theme_light` = light).
    pub sync_os_theme: bool,
    /// Window background opacity, 0.3–1.0.
    pub window_opacity: f32,
    pub input_position: InputPosition,
    pub tab_layout: TabLayout,
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
    /// Ask before pasting text containing a newline into a raw program that
    /// has not enabled bracketed paste (each newline would press Enter).
    pub confirm_multiline_paste: bool,
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
    /// Toasts, desktop notifications and sounds per event (`[notifications]`;
    /// tables stay after the plain keys so the TOML writer accepts them).
    pub notifications: crate::notifications::Notifications,
    /// Named ssh hosts (`[[ssh_profiles]]`); ~/.ssh/config hosts are added
    /// to the palette automatically.
    pub ssh_profiles: Vec<crate::ssh::SshProfile>,
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
            save_transcripts: false,
            durable_sessions: false,
            control_api: "navigate".into(),
            shell: None,
            theme_light: "light-default".into(),
            sync_os_theme: false,
            window_opacity: 1.0,
            input_position: InputPosition::Bottom,
            tab_layout: TabLayout::Horizontal,
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
            confirm_multiline_paste: true,
            working_directory: "previous".into(),
            alias_expansion: false,
            error_underline: true,
            command_corrections: true,
            global_hotkey: String::new(),
            option_as_meta: false,
            mouse_reporting: true,
            notifications: Default::default(),
            ssh_profiles: Vec::new(),
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

/// Top-level `Option` settings: absent from a serialized default, so
/// `merge_table` cannot tell them from unknown keys without this list.
const OPTIONAL_KEYS: &[&str] = &["shell"];

/// Make `doc` hold `new` (see `Config::edit_toml`). `default` is the same
/// table for a default config; an empty `default` marks a map such as
/// `[keybindings]`, whose entries are exactly those of `new`.
fn merge_table(
    doc: &mut dyn toml_edit::TableLike,
    new: &toml::Table,
    default: &toml::Table,
    optional: &[&str],
) {
    let is_map = default.is_empty();
    let stale: Vec<String> = doc
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| {
            !new.contains_key(k)
                && (is_map || default.contains_key(k) || optional.contains(&k.as_str()))
        })
        .collect();
    for k in stale {
        doc.remove(&k);
    }
    let empty = toml::Table::new();
    for (k, v) in new {
        let dv = default.get(k);
        let sub_default = dv.and_then(toml::Value::as_table).unwrap_or(&empty);
        match (doc.get_mut(k), v) {
            (Some(item), toml::Value::Table(nt)) if item.is_table_like() => {
                if let Some(t) = item.as_table_like_mut() {
                    merge_table(t, nt, sub_default, &[]);
                }
            }
            (Some(item), _) => {
                if plain_value(k, item).as_ref() != Some(v)
                    && let Some(mut fresh) = edit_item(k, v)
                {
                    // keep the value's trailing comment and spacing
                    if let (Some(old), Some(fresh_v)) = (item.as_value(), fresh.as_value_mut()) {
                        *fresh_v.decor_mut() = old.decor().clone();
                    }
                    *item = fresh;
                }
            }
            (None, toml::Value::Table(nt)) => {
                let mut t = toml_edit::Table::new();
                merge_table(&mut t, nt, sub_default, &[]);
                if !t.is_empty() {
                    doc.insert(k, toml_edit::Item::Table(t));
                }
            }
            (None, _) => {
                if dv != Some(v)
                    && let Some(fresh) = edit_item(k, v)
                {
                    doc.insert(k, fresh);
                }
            }
        }
    }
}

/// `item` (stored under `key`) as a plain TOML value, for comparison.
fn plain_value(key: &str, item: &toml_edit::Item) -> Option<toml::Value> {
    let mut d = toml_edit::DocumentMut::new();
    d.insert(key, item.clone());
    toml::from_str::<toml::Table>(&d.to_string())
        .ok()?
        .remove(key)
}

/// `value` as an editable item, formatted like `toml::to_string_pretty`.
fn edit_item(key: &str, value: &toml::Value) -> Option<toml_edit::Item> {
    let mut t = toml::Table::new();
    t.insert(key.to_string(), value.clone());
    let mut d: toml_edit::DocumentMut = toml::to_string_pretty(&t).ok()?.parse().ok()?;
    d.remove(key)
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

pub fn themes_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("themes"))
}

impl Config {
    /// Read `config.toml`. `Ok(None)`: no file yet (first run, defaults
    /// apply). `Err`: the file exists but cannot be read or parsed — the
    /// caller keeps its last good config rather than resetting to defaults,
    /// which would silently re-enable `control_api`.
    pub fn try_load() -> Result<Option<Config>, String> {
        match config_path() {
            Some(path) => Self::try_load_from(&path),
            None => Ok(None),
        }
    }

    pub fn try_load_from(path: &Path) -> Result<Option<Config>, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)
                .map(Some)
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Settings for a config file that exists but is invalid at startup:
    /// defaults, except automation stays off until the file is fixed.
    pub fn fallback() -> Config {
        Config {
            control_api: "off".into(),
            ..Config::default()
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = config_path() else {
            return Err(std::io::Error::other("no config dir"));
        };
        self.save_to(&path)
    }

    /// Edit the file in place (see `edit_toml`), then replace it atomically:
    /// write a sibling temp file, flush it, keep the old file's
    /// permissions, rename over the target. A failed write never truncates
    /// the existing config, and the hot-reload watcher never reads a
    /// half-written file.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let dir = path
            .parent()
            .ok_or_else(|| std::io::Error::other("config path has no parent"))?;
        std::fs::create_dir_all(dir)?;
        let existing = match std::fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let body = self
            .edit_toml(existing.as_deref())
            .map_err(std::io::Error::other)?;
        let tmp = dir.join(format!(".config.toml.tmp-{}", std::process::id()));
        let result = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(body.as_bytes())?;
            f.sync_all()?;
            if let Ok(meta) = std::fs::metadata(path) {
                std::fs::set_permissions(&tmp, meta.permissions())?;
            }
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// The user's file (`existing`) changed to hold this config. A changed
    /// value is rewritten in place, keeping its comments; a setting the
    /// file never mentioned is added only when it differs from the
    /// default; comments, order and unknown keys stay. A file that does not
    /// parse is never rewritten. Should the edit not read back as exactly
    /// this config, the whole file is written instead.
    pub fn edit_toml(&self, existing: Option<&str>) -> Result<String, String> {
        let mut doc: toml_edit::DocumentMut = existing
            .unwrap_or("")
            .parse()
            .map_err(|e| format!("config.toml does not parse, not overwriting it: {e}"))?;
        // a file of only comments keeps them on top, above the new settings
        let header = if doc.as_table().is_empty() {
            let text = doc.trailing().as_str().unwrap_or_default().to_string();
            doc.set_trailing("");
            text
        } else {
            String::new()
        };
        let table = |c: &Config| match toml::Value::try_from(c) {
            Ok(toml::Value::Table(t)) => Ok(t),
            Ok(_) => Err("config is not a table".to_string()),
            Err(e) => Err(e.to_string()),
        };
        merge_table(
            doc.as_table_mut(),
            &table(self)?,
            &table(&Config::default())?,
            OPTIONAL_KEYS,
        );
        let out = format!("{header}{doc}");
        // compared as TOML text: an accepted `nan` never equals itself as
        // f32, and the lossy fallback would drop comments and unknown keys
        let same = |back: &Config| match (toml::to_string(back), toml::to_string(self)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        match toml::from_str::<Config>(&out) {
            Ok(back) if same(&back) => Ok(out),
            _ => toml::to_string_pretty(self).map_err(|e| e.to_string()),
        }
    }

    /// Font size in points: non-finite falls back to the default, then
    /// clamps to the supported range.
    pub fn font_size(&self) -> f32 {
        if self.font_size.is_finite() {
            self.font_size.clamp(8.0, 28.0)
        } else {
            13.0
        }
    }

    /// Line-height multiplier, validated like `font_size()`.
    pub fn line_height(&self) -> f32 {
        if self.line_height.is_finite() {
            self.line_height.clamp(1.0, 2.2)
        } else {
            1.65
        }
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
        c.notifications.sound_failed = "Basso".into();
        c.notifications.toasts = false;
        let s = toml::to_string_pretty(&c).unwrap();
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

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shift-config-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn invalid_file_is_an_error_not_defaults() {
        let dir = scratch("invalid");
        let path = dir.join("config.toml");
        assert_eq!(Config::try_load_from(&path), Ok(None));
        std::fs::write(&path, "control_api = \"off\"\nfont_size = 19.0\n").unwrap();
        let good = Config::try_load_from(&path).unwrap().unwrap();
        assert_eq!(good.control_api, "off");
        std::fs::write(&path, "font_size = [\n").unwrap();
        let err = Config::try_load_from(&path).unwrap_err();
        assert!(err.contains("config.toml"), "{err}");
        assert_eq!(Config::fallback().control_api, "off");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_is_atomic_under_concurrent_reads() {
        let dir = scratch("atomic");
        let path = dir.join("config.toml");
        let mut c = Config::default();
        for i in 0..400 {
            c.keybindings
                .insert(format!("action_{i}"), format!("cmd+{i}"));
        }
        c.save_to(&path).unwrap();
        let reader_path = path.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader_stop = stop.clone();
        let reader = std::thread::spawn(move || {
            let mut bad = 0;
            while !reader_stop.load(std::sync::atomic::Ordering::Relaxed) {
                if Config::try_load_from(&reader_path).ok().flatten().is_none() {
                    bad += 1;
                }
            }
            bad
        });
        for _ in 0..200 {
            c.save_to(&path).unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(reader.join().unwrap(), 0, "a reader saw a partial config");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name() != "config.toml")
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_save_keeps_the_original_bytes() {
        let dir = scratch("failsave");
        let path = dir.join("config.toml");
        std::fs::write(&path, "# mine\nfont_size = 19.0\n").unwrap();
        // a directory occupying the temp file's name makes the write fail
        std::fs::create_dir(dir.join(format!(".config.toml.tmp-{}", std::process::id()))).unwrap();
        assert!(Config::default().save_to(&path).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\nfont_size = 19.0\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_edits_in_place_keeping_comments_and_unknown_keys() {
        let file = "# my setup\nfont_size = 19.0 # big\ntheme = \"dark-default\"\nfuture_option = 1\n\n# bindings\n[keybindings]\nnew_tab = \"cmd+t\"\ncopy = \"cmd+c\"\n";
        let mut c: Config = toml::from_str(file).unwrap();
        c.font_size = 15.0;
        c.keybindings.remove("copy");
        let out = c.edit_toml(Some(file)).unwrap();
        assert_eq!(
            out,
            "# my setup\nfont_size = 15.0 # big\ntheme = \"dark-default\"\nfuture_option = 1\n\n# bindings\n[keybindings]\nnew_tab = \"cmd+t\"\n"
        );
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c);
    }

    #[test]
    fn save_adds_only_changed_settings() {
        assert_eq!(Config::default().edit_toml(None).unwrap(), "");
        let mut c = Config {
            font_size: 14.0,
            shell: Some("/bin/zsh".into()),
            ..Default::default()
        };
        c.notifications.toasts = false;
        let out = c.edit_toml(Some("# hi\n")).unwrap();
        assert!(out.starts_with("# hi\n"), "{out}");
        assert!(out.contains("font_size = 14.0"), "{out}");
        assert!(out.contains("[notifications]\ntoasts = false\n"), "{out}");
        assert!(!out.contains("theme"), "unchanged defaults stay out: {out}");
        assert_eq!(toml::from_str::<Config>(&out).unwrap(), c);
        // clearing an optional setting removes its line
        let cleared = Config {
            shell: None,
            ..c.clone()
        };
        let out = cleared.edit_toml(Some(&out)).unwrap();
        assert!(!out.contains("shell"), "{out}");
    }

    #[test]
    fn save_refuses_to_rewrite_an_unparseable_file() {
        assert!(
            Config::default()
                .edit_toml(Some("font_size = [\n"))
                .is_err()
        );
    }

    #[test]
    fn font_metrics_reject_non_finite() {
        let c: Config = toml::from_str("font_size = nan\nline_height = inf").unwrap();
        assert_eq!(c.font_size(), 13.0);
        assert_eq!(c.line_height(), 1.65);
        let c = Config {
            font_size: 99.0,
            line_height: 0.1,
            ..Default::default()
        };
        assert_eq!(c.font_size(), 28.0);
        assert_eq!(c.line_height(), 1.0);
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

    #[test]
    fn save_keeps_comments_next_to_non_finite_floats() {
        for key in [
            "font_size",
            "line_height",
            "window_padding",
            "window_opacity",
        ] {
            for value in ["nan", "inf", "-inf", "13.0"] {
                let file = format!(
                    "# keep me\n{key} = {value}\nfuture_option = 42\ncontrol_api = \"off\"\n"
                );
                let mut c: Config = toml::from_str(&file).unwrap();
                c.notifications.toasts = !c.notifications.toasts;
                let out = c.edit_toml(Some(&file)).unwrap();
                let case = format!("{key} = {value}");
                assert!(out.starts_with("# keep me\n"), "{case}: {out}");
                assert!(out.contains("future_option = 42"), "{case}: {out}");
                assert!(out.contains(&case), "{case}: {out}");
                let back: Config = toml::from_str(&out).unwrap();
                assert_eq!(back.control_api, "off", "{case}");
                assert_eq!(back.notifications.toasts, c.notifications.toasts, "{case}");
                assert_eq!(back.font_size(), c.font_size(), "{case}");
                assert_eq!(back.line_height(), c.line_height(), "{case}");
            }
        }
    }
}
