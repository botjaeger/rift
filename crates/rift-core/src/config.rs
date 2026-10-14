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

/// Where blocks start while they do not fill the pane, with the input at
/// the bottom: right above the input (Warp "Pin to the bottom"), or at the
/// top of the pane, the gap left above the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BlocksStart {
    #[default]
    Bottom,
    Top,
}

/// One context chip of the Rift prompt (Warp "Edit prompt"). A chip with
/// nothing to show (no repository, no virtualenv) is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptChip {
    /// Working directory (click: change directory).
    Cwd,
    /// Branch with ahead/behind and the dirty dot (click: change branch).
    GitBranch,
    /// Changed files and lines: `3 • +10 −2`.
    GitDiff,
    User,
    Host,
    /// Python virtualenv, else the version `python3` runs (click: switch
    /// this shell's version).
    Python,
    /// The version `php` runs (click: switch this shell's version).
    Php,
    /// The checked-out branch's pull request, via `gh` (click: open it).
    GithubPr,
    /// The Docker Compose project around the cwd and its running containers
    /// (click: list them).
    Docker,
    Date,
    /// 12-hour time (`03:48 pm`).
    Time12,
    /// 24-hour time (`15:48`).
    Time24,
    /// Time since the tab's first command (`2h 05m`); hidden until then.
    Elapsed,
}

impl PromptChip {
    pub const ALL: [PromptChip; 13] = [
        PromptChip::User,
        PromptChip::Host,
        PromptChip::Cwd,
        PromptChip::GitBranch,
        PromptChip::GitDiff,
        PromptChip::GithubPr,
        PromptChip::Python,
        PromptChip::Php,
        PromptChip::Docker,
        PromptChip::Date,
        PromptChip::Time12,
        PromptChip::Time24,
        PromptChip::Elapsed,
    ];

    /// The default prompt: the design's cwd and branch chips, then the diff
    /// stats (Warp's default; hidden on a clean tree).
    pub fn defaults() -> Vec<PromptChip> {
        vec![PromptChip::Cwd, PromptChip::GitBranch, PromptChip::GitDiff]
    }

    pub fn label(self) -> &'static str {
        match self {
            PromptChip::Cwd => "Working directory",
            PromptChip::GitBranch => "Git branch",
            PromptChip::GitDiff => "Git diff stats",
            PromptChip::User => "User",
            PromptChip::Host => "Host",
            PromptChip::Python => "Python environment",
            PromptChip::Php => "PHP version",
            PromptChip::GithubPr => "GitHub pull request",
            PromptChip::Docker => "Docker Compose",
            PromptChip::Date => "Date",
            PromptChip::Time12 => "Time (12-hour)",
            PromptChip::Time24 => "Time (24-hour)",
            PromptChip::Elapsed => "Time on this tab",
        }
    }

    /// What the chip shows and what a click on it does (its hover card).
    pub fn about(self) -> &'static str {
        match self {
            PromptChip::Cwd => {
                "The pane's working directory. Click to move into a subdirectory or up to a parent."
            }
            PromptChip::GitBranch => {
                "The checked-out branch, commits ahead (↑) and behind (↓) its upstream, and • \
                 when there are uncommitted changes. Click to switch to another branch or \
                 create one."
            }
            PromptChip::GitDiff => {
                "Files changed in the working tree, and the lines added and removed. Click to \
                 open the code review panel."
            }
            PromptChip::User => "The user you are logged in as.",
            PromptChip::Host => "This machine's host name.",
            PromptChip::Python => {
                "The active virtualenv, else the Python version python3 runs here. Click to \
                 switch this shell's Python (pyenv versions and installed interpreters)."
            }
            PromptChip::Php => {
                "The PHP version php runs here. Click to switch this shell's PHP between the \
                 installed versions."
            }
            PromptChip::GithubPr => {
                "The pull request of the checked-out branch, from the GitHub CLI (gh). Click \
                 to open it in your browser."
            }
            PromptChip::Docker => {
                "The Docker Compose project around this directory and how many of its \
                 containers run. Click to see each container's ports, uptime and health, \
                 and to follow a service's logs."
            }
            PromptChip::Date => "Today's date.",
            PromptChip::Time12 => "The current time, 12-hour.",
            PromptChip::Time24 => "The current time, 24-hour.",
            PromptChip::Elapsed => {
                "How long ago the first command ran in this tab: the time worked on it. \
                 Shows once a command has run, and survives a restart."
            }
        }
    }
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

/// What renders above the editor: Rift's context chips, or the shell's
/// own prompt (PS1 / starship / p10k) left visible (Warp "Use my shell's
/// prompt").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    #[default]
    #[serde(alias = "shift", alias = "runway")]
    Rift,
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

/// An f32 written as its shortest decimal (`1.6`, not the f64 widening
/// `1.600000023841858`).
fn short_f32<S: serde::Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(v.to_string().parse().unwrap_or(f64::from(*v)))
}

/// A u8 setting read from any integer, saturating at 0 / 255 (the getter
/// clamps further): an out-of-range number must not reject the whole file.
fn saturating_u8<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let v = i64::deserialize(d)?;
    Ok(v.clamp(0, 255) as u8)
}

/// User configuration — `~/.config/rift/config.toml`, hot-reloaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font_family: String,
    #[serde(serialize_with = "short_f32")]
    pub font_size: f32,
    /// Multiplier (1.2, as in Warp; the design spec drew 1.65).
    #[serde(serialize_with = "short_f32")]
    pub line_height: f32,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    /// Theme name: builtin (`dark-default`, `catppuccin-mocha`, `tokyo-night`)
    /// or a file stem in `~/.config/rift/themes/`.
    pub theme: String,
    pub scrollback: usize,
    #[serde(serialize_with = "short_f32")]
    pub window_padding: f32,
    pub ligatures: bool,
    /// Reopen last session's tabs (with cwds) on launch.
    pub restore_session: bool,
    /// Also save finished blocks (command, status, output) and show them
    /// again on relaunch. Output may contain secrets, so this is opt-in.
    pub save_transcripts: bool,
    /// Shells run in the per-user session service, so closing or quitting
    /// Rift leaves them running and relaunching reattaches them (on by
    /// default; `durable_sessions = false` keeps shells in the window).
    pub durable_sessions: bool,
    /// Look for a new release at launch and every 6 hours (`update.rs`).
    pub check_for_updates: bool,
    /// Local control socket: "off", "navigate" (list/status/focus, and
    /// prepare-restart, honored only with the token of an update in flight)
    /// or "full" (also split/open/send-text). See control.rs.
    pub control_api: String,
    /// Paired phones may reach this user's windows through the remote
    /// gateway (`rift-terminal remote --managed`, which the windows start
    /// while this is on; docs/remote-protocol.md). Off by default.
    pub remote_access: bool,
    /// Address the remote gateway listens on (`IP:PORT`; `0.0.0.0` = every
    /// network interface).
    pub remote_listen: String,
    /// Relay the remote gateway also registers at, for phones outside the
    /// local network (`wss://…`; `ws://` only for local relays). Defaults to
    /// the public `DEFAULT_RELAY`; empty: no relay. The relay only ever sees
    /// encrypted traffic.
    pub remote_relay: String,
    /// Reach the gateway from any network without a server: it runs a relay
    /// on this computer behind a Cloudflare quick tunnel (`cloudflared`)
    /// and registers there instead of at `remote_relay`. The tunnel's
    /// address changes on every start, so phones pair again after one.
    pub remote_tunnel: bool,
    pub shell: Option<String>,
    /// Theme used while the OS is in light mode when `sync_os_theme` is on.
    pub theme_light: String,
    /// Follow the OS light/dark appearance (`theme` = dark, `theme_light` = light).
    pub sync_os_theme: bool,
    /// Window background opacity, 0.01–1.0 (Warp: 1–100 %).
    #[serde(serialize_with = "short_f32")]
    pub window_opacity: f32,
    /// macOS background blur radius behind the window, 1–64 px (Warp:
    /// default 1, i.e. none). Visible only below full opacity.
    #[serde(deserialize_with = "saturating_u8")]
    pub window_blur: u8,
    pub input_position: InputPosition,
    pub blocks_start: BlocksStart,
    pub tab_layout: TabLayout,
    pub prompt: PromptMode,
    /// The Rift prompt's chips, in order (`prompt = "rift"`).
    pub prompt_chips: Vec<PromptChip>,
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
    /// Alert (toast, and a desktop notification while the window is not
    /// focused) when a long command finishes.
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
    /// macOS: the right Option key's Meta setting; unset = same as
    /// `option_as_meta` (which then covers only the left key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_option_as_meta: Option<bool>,
    /// Middle-click pastes the clipboard into the pane under the pointer
    /// (unless the program there takes mouse clicks).
    pub middle_click_paste: bool,
    /// Lines one mouse-wheel notch scrolls the terminal (1–20; trackpads
    /// scroll by pixels).
    pub wheel_scroll_lines: u8,
    /// OSC 52 clipboard access for programs: `"off"`, `"write"` (copy only)
    /// or `"read_write"` (also read it back: any program or remote host can
    /// then read your clipboard).
    pub clipboard_access: String,
    /// Forward mouse clicks, drags and the wheel to apps that enable mouse
    /// reporting (vim, tmux, htop). Hold Shift to select text instead.
    /// Focus in/out reports (mode 1004) do not depend on it.
    pub mouse_reporting: bool,
    /// Where ⌘-clicked file paths and file links open: `"system"` (the
    /// file's default app), `"rift"` (the viewer pane) or an editor id
    /// from `editors::EDITORS` (`"vscode"`, `"zed"`, …).
    pub link_editor: String,
    /// Where files from the explorer and code review (↗) open: `"rift"`
    /// (the viewer pane), `"system"` or an editor id.
    pub file_editor: String,
    /// The file explorer lists dotfiles (`.env`, `.github/`); `.git` and
    /// git-ignored entries stay hidden either way.
    pub explorer_hidden_files: bool,
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
            line_height: 1.2,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            theme: "dark-default".into(),
            scrollback: 10_000,
            window_padding: 8.0,
            ligatures: true,
            restore_session: true,
            save_transcripts: false,
            durable_sessions: true,
            check_for_updates: true,
            control_api: "navigate".into(),
            remote_access: false,
            remote_listen: "0.0.0.0:7878".into(),
            remote_relay: DEFAULT_RELAY.into(),
            remote_tunnel: false,
            shell: None,
            theme_light: "light-default".into(),
            sync_os_theme: false,
            window_opacity: 1.0,
            window_blur: 1,
            input_position: InputPosition::Bottom,
            blocks_start: BlocksStart::Bottom,
            tab_layout: TabLayout::Horizontal,
            prompt: PromptMode::Rift,
            prompt_chips: PromptChip::defaults(),
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
            right_option_as_meta: None,
            middle_click_paste: true,
            wheel_scroll_lines: 3,
            clipboard_access: "write".into(),
            mouse_reporting: true,
            link_editor: crate::editors::SYSTEM.into(),
            file_editor: crate::editors::RIFT.into(),
            explorer_hidden_files: true,
            notifications: Default::default(),
            ssh_profiles: Vec::new(),
            keybindings: BTreeMap::new(),
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// `config_dir` over explicit env values; an empty `XDG_CONFIG_HOME` counts
/// as unset (a relative path would put the config in the working directory).
fn config_dir_from(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    xdg.filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| PathBuf::from(h).join(".config")))
        .map(|root| app_dir(&root))
}

/// Rift's directory under a config or data root: `rift`, or the directory of
/// an earlier name of the app (`shift`, then `runway`) while `rift` does not
/// exist.
pub(crate) fn app_dir(root: &Path) -> PathBuf {
    let current = root.join("rift");
    if current.exists() {
        return current;
    }
    ["shift", "runway"]
        .into_iter()
        .map(|name| root.join(name))
        .find(|legacy| legacy.exists())
        .unwrap_or(current)
}

/// Expand a leading `~` or `~/`; `~root` stays a literal name.
fn expand_tilde(path: &str, home: Option<&Path>) -> PathBuf {
    match home {
        Some(h) if path == "~" => h.to_path_buf(),
        Some(h) if path.starts_with("~/") => h.join(path[2..].trim_start_matches('/')),
        _ => PathBuf::from(path),
    }
}

/// The public relay `remote_relay` defaults to: phones reach a computer from
/// any network with nothing to set up. It forwards only Noise ciphertext.
pub const DEFAULT_RELAY: &str = "wss://rift-relay.playhaws.com";

/// Set in a scripted run (and inherited by the gateway it starts): the
/// default relay is not used, so tests never reach the public relay. A relay
/// the config names explicitly still is.
pub const NO_DEFAULT_RELAY_ENV: &str = "RIFT_NO_DEFAULT_RELAY";

fn relay_of(setting: &str, no_default: bool) -> Option<&str> {
    let relay = setting.trim();
    (!relay.is_empty() && !(no_default && relay == DEFAULT_RELAY)).then_some(relay)
}

/// Top-level `Option` settings: absent from a serialized default, so
/// `merge_table` cannot tell them from unknown keys without this list.
const OPTIONAL_KEYS: &[&str] = &["shell", "right_option_as_meta"];

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

/// `path` for display, with the home directory shortened to `~`.
pub fn tilde_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

pub fn themes_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("themes"))
}

impl Config {
    /// The relay the gateway registers at: `remote_relay` trimmed, `None`
    /// when empty, and `None` for the default relay under
    /// `RIFT_NO_DEFAULT_RELAY`.
    pub fn relay(&self) -> Option<&str> {
        relay_of(
            &self.remote_relay,
            std::env::var_os(NO_DEFAULT_RELAY_ENV).is_some(),
        )
    }

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
            Ok(s) => toml::from_str::<Config>(&s)
                .map(|c| Some(c.migrated()))
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Values written under an earlier name of the app: the viewer pane was
    /// `"shift"` before it was `editors::RIFT`.
    fn migrated(mut self) -> Config {
        for editor in [&mut self.link_editor, &mut self.file_editor] {
            if editor.trim() == "shift" {
                *editor = crate::editors::RIFT.into();
            }
        }
        self
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
        // a symlinked config.toml (dotfiles, stow) is edited through the
        // link: rename onto the link itself would replace it with a file
        let resolved;
        let path = match std::fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_symlink() => {
                resolved = std::fs::canonicalize(path)?;
                resolved.as_path()
            }
            _ => path,
        };
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
            1.2
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
            path => Some(expand_tilde(path, home.as_deref())),
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

    /// Scrollback lines clamped to 0–1,000,000 (a hand-edited file can hold
    /// any `usize`, and the terminal allocates per line).
    pub fn scrollback_lines(&self) -> usize {
        self.scrollback.min(1_000_000)
    }

    /// Terminal lines per wheel notch, clamped to 1–20.
    pub fn wheel_lines(&self) -> f32 {
        self.wheel_scroll_lines.clamp(1, 20) as f32
    }

    /// The right Option key sends Meta (macOS).
    pub fn right_option_meta(&self) -> bool {
        self.right_option_as_meta.unwrap_or(self.option_as_meta)
    }

    /// Window opacity clamped to the supported range. The floor is Warp's
    /// 1 %: at 0 Metal shows the previous frame's pixels.
    pub fn opacity(&self) -> f32 {
        if self.window_opacity.is_finite() {
            self.window_opacity.clamp(0.01, 1.0)
        } else {
            1.0
        }
    }

    /// Background blur radius clamped to Warp's 1–64 px.
    pub fn blur(&self) -> u8 {
        self.window_blur.clamp(1, 64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsetting_an_optional_key_edits_only_that_line() {
        let existing =
            "# keep\nfuture_option = 1\nright_option_as_meta = true\nshell = \"/bin/sh\"\n";
        let out = Config::default().edit_toml(Some(existing)).unwrap();
        assert_eq!(out, "# keep\nfuture_option = 1\n");
    }

    #[test]
    fn floats_write_short_and_a_huge_blur_still_loads() {
        let c = Config {
            line_height: 1.6,
            window_opacity: 0.51,
            ..Default::default()
        };
        let text = toml::to_string(&c).unwrap();
        assert!(text.contains("line_height = 1.6\n"), "{text}");
        assert!(text.contains("window_opacity = 0.51\n"), "{text}");
        let big: Config = toml::from_str("window_blur = 300\nfont_size = 15.0\n").unwrap();
        assert_eq!(
            (big.window_blur, big.blur(), big.font_size),
            (255, 64, 15.0)
        );
        let neg: Config = toml::from_str("window_blur = -4\n").unwrap();
        assert_eq!(neg.blur(), 1);
    }

    #[test]
    fn wheel_lines_clamp_and_right_option_follows_left_until_set() {
        let mut c = Config::default();
        assert_eq!(c.wheel_lines(), 3.0);
        c.wheel_scroll_lines = 0;
        assert_eq!(c.wheel_lines(), 1.0);
        c.wheel_scroll_lines = 200;
        assert_eq!(c.wheel_lines(), 20.0);
        c.option_as_meta = true;
        assert!(c.right_option_meta());
        c.right_option_as_meta = Some(false);
        assert!(!c.right_option_meta());
        // unset stays out of config.toml; set is written
        let unset = toml::to_string(&Config::default()).unwrap();
        assert!(!unset.contains("right_option_as_meta"), "{unset}");
        assert!(
            toml::to_string(&c)
                .unwrap()
                .contains("right_option_as_meta = false")
        );
    }

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
    fn remote_access_is_off_unless_a_real_boolean_turns_it_on() {
        let c = Config::default();
        assert!(!c.remote_access);
        assert_eq!(c.remote_listen, "0.0.0.0:7878");
        let c: Config =
            toml::from_str("remote_access = true\nremote_listen = \"127.0.0.1:9\"\n").unwrap();
        assert!(c.remote_access);
        assert_eq!(c.remote_listen, "127.0.0.1:9");
        // under another table it is not the setting
        let c: Config = toml::from_str("[x]\nremote_access = true\n").unwrap();
        assert!(!c.remote_access);
        for bad in ["remote_access = \"true\"", "remote_access = 1"] {
            assert!(toml::from_str::<Config>(bad).is_err(), "{bad}");
        }
        // the invalid-file fallback keeps it off too
        assert!(!Config::fallback().remote_access);
        // Settings writes only the changed key, in place
        let on = Config {
            remote_access: true,
            ..Default::default()
        };
        let out = on.edit_toml(Some("# mine\nfont_size = 13.0\n")).unwrap();
        assert_eq!(out, "# mine\nfont_size = 13.0\nremote_access = true\n");
        let off = Config {
            remote_access: false,
            ..toml::from_str(&out).unwrap()
        };
        let out = off.edit_toml(Some(&out)).unwrap();
        assert_eq!(out, "# mine\nfont_size = 13.0\nremote_access = false\n");
    }

    #[test]
    fn remote_relay_defaults_to_the_public_relay_and_is_saved_in_place() {
        let c = Config::default();
        assert_eq!(c.remote_relay, DEFAULT_RELAY);
        assert!(DEFAULT_RELAY.starts_with("wss://"));
        // a file without the key gets the default; an empty value opts out
        let c: Config = toml::from_str("font_size = 13.0\n").unwrap();
        assert_eq!(c.remote_relay, DEFAULT_RELAY);
        let c: Config = toml::from_str("remote_relay = \"\"\n").unwrap();
        assert_eq!(c.remote_relay, "");
        let c: Config = toml::from_str("remote_relay = \"wss://relay.example.com\"\n").unwrap();
        assert_eq!(c.remote_relay, "wss://relay.example.com");
        assert!(toml::from_str::<Config>("remote_relay = true").is_err());
        // the default is not written; another one is, in place
        let out = Config::default()
            .edit_toml(Some("# mine\nfont_size = 13.0\n"))
            .unwrap();
        assert_eq!(out, "# mine\nfont_size = 13.0\n");
        let set = Config {
            remote_relay: "ws://127.0.0.1:18790".into(),
            ..Default::default()
        };
        let out = set.edit_toml(Some(&out)).unwrap();
        assert_eq!(
            out,
            "# mine\nfont_size = 13.0\nremote_relay = \"ws://127.0.0.1:18790\"\n"
        );
        // clearing it writes the empty value back over the old one
        let cleared = Config {
            remote_relay: String::new(),
            ..toml::from_str(&out).unwrap()
        };
        let out = cleared.edit_toml(Some(&out)).unwrap();
        assert_eq!(out, "# mine\nfont_size = 13.0\nremote_relay = \"\"\n");
        assert_eq!(toml::from_str::<Config>(&out).unwrap().remote_relay, "");
    }

    #[test]
    fn scripted_runs_skip_only_the_default_relay() {
        assert_eq!(relay_of(DEFAULT_RELAY, false), Some(DEFAULT_RELAY));
        assert_eq!(relay_of(DEFAULT_RELAY, true), None);
        assert_eq!(relay_of(&format!(" {DEFAULT_RELAY} "), true), None);
        assert_eq!(
            relay_of(" ws://127.0.0.1:9 ", true),
            Some("ws://127.0.0.1:9")
        );
        assert_eq!(relay_of("  ", false), None);
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
        assert_eq!(c.blocks_start, BlocksStart::Bottom);
        let top: Config = toml::from_str("blocks_start = \"top\"").unwrap();
        assert_eq!(top.blocks_start, BlocksStart::Top);
        assert_eq!(c.prompt_chips, PromptChip::defaults());
        let chips: Config =
            toml::from_str("prompt_chips = [\"time24\", \"cwd\", \"git_diff\"]").unwrap();
        assert_eq!(
            chips.prompt_chips,
            [PromptChip::Time24, PromptChip::Cwd, PromptChip::GitDiff]
        );
        assert!(toml::from_str::<Config>("prompt_chips = [\"weather\"]").is_err());
        assert_eq!(c.prompt, PromptMode::Shell);
        assert_eq!(c.bell, Bell::None);
        assert!(c.vim_mode);
        assert!(toml::from_str::<Config>("input_position = \"middle\"").is_err());
        let legacy: Config = toml::from_str("prompt = \"runway\"").unwrap();
        assert_eq!(legacy.prompt, PromptMode::Rift);
        let legacy: Config = toml::from_str("prompt = \"shift\"").unwrap();
        assert_eq!(legacy.prompt, PromptMode::Rift);
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
    fn start_dir_expands_only_tilde_and_tilde_slash() {
        let h = Path::new("/home/u");
        assert_eq!(expand_tilde("~", Some(h)), PathBuf::from("/home/u"));
        assert_eq!(expand_tilde("~/", Some(h)), PathBuf::from("/home/u"));
        assert_eq!(expand_tilde("~/a/b", Some(h)), PathBuf::from("/home/u/a/b"));
        // `~root` is a literal name, never $HOME/root
        assert_eq!(expand_tilde("~root", Some(h)), PathBuf::from("~root"));
        assert_eq!(expand_tilde("/abs", Some(h)), PathBuf::from("/abs"));
        assert_eq!(expand_tilde("~/a", None), PathBuf::from("~/a"));
    }

    #[test]
    fn empty_xdg_config_home_is_unset() {
        use std::ffi::OsString;
        let home = Some(OsString::from("/home/u"));
        let want = Some(PathBuf::from("/home/u/.config/rift"));
        assert_eq!(config_dir_from(Some(OsString::new()), home.clone()), want);
        assert_eq!(config_dir_from(None, home.clone()), want);
        assert_eq!(
            config_dir_from(Some(OsString::from("/x")), home),
            Some(PathBuf::from("/x/rift"))
        );
    }

    #[test]
    fn tilde_path_shortens_only_the_home_prefix() {
        let h = Some(Path::new("/home/u"));
        assert_eq!(
            tilde_path(Path::new("/home/u/.config/rift/config.toml"), h),
            "~/.config/rift/config.toml"
        );
        assert_eq!(tilde_path(Path::new("/home/u"), h), "~");
        assert_eq!(tilde_path(Path::new("/home/user2/x"), h), "/home/user2/x");
        assert_eq!(tilde_path(Path::new("/etc/x"), None), "/etc/x");
    }

    #[test]
    fn earlier_app_names_keep_their_config_dir() {
        let root = std::env::temp_dir().join(format!("rift-legacy-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let xdg = Some(root.clone().into_os_string());
        let dir = |name: &str| Some(root.join(name));
        std::fs::create_dir_all(root.join("runway")).unwrap();
        assert_eq!(config_dir_from(xdg.clone(), None), dir("runway"));
        std::fs::create_dir_all(root.join("shift")).unwrap();
        assert_eq!(config_dir_from(xdg.clone(), None), dir("shift"));
        std::fs::create_dir_all(root.join("rift")).unwrap();
        assert_eq!(config_dir_from(xdg, None), dir("rift"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn shift_viewer_value_loads_as_rift() {
        let path =
            std::env::temp_dir().join(format!("rift-legacy-editor-{}.toml", std::process::id()));
        std::fs::write(&path, "link_editor = \"shift\"\nfile_editor = \"zed\"\n").unwrap();
        let c = Config::try_load_from(&path).unwrap().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(c.link_editor, crate::editors::RIFT);
        assert_eq!(c.file_editor, "zed");
    }

    #[cfg(unix)]
    #[test]
    fn save_through_symlink_keeps_the_link() {
        let dir = scratch("symlink");
        let real_dir = dir.join("dotfiles");
        std::fs::create_dir_all(&real_dir).unwrap();
        let real = real_dir.join("config.toml");
        std::fs::write(&real, "# mine\nfont_size = 15.0\n").unwrap();
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let c = Config {
            font_size: 17.0,
            ..Config::default()
        };
        c.save_to(&link).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "link replaced by a regular file"
        );
        let body = std::fs::read_to_string(&real).unwrap();
        assert!(body.contains("# mine") && body.contains("17"), "{body}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scrollback_lines_is_bounded() {
        let c = Config {
            scrollback: usize::MAX,
            ..Config::default()
        };
        assert_eq!(c.scrollback_lines(), 1_000_000);
        assert_eq!(Config::default().scrollback_lines(), 10_000);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rift-config-{tag}-{}", std::process::id()));
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
        assert_eq!(c.line_height(), 1.2);
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
        assert_eq!(c.opacity(), 0.1);
        c.window_opacity = 0.0;
        assert_eq!(c.opacity(), 0.01);
        c.window_opacity = 7.0;
        assert_eq!(c.opacity(), 1.0);
        c.window_opacity = f32::NAN;
        assert_eq!(c.opacity(), 1.0);
        assert_eq!(c.blur(), 1);
        c.window_blur = 0;
        assert_eq!(c.blur(), 1);
        c.window_blur = 200;
        assert_eq!(c.blur(), 64);
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
