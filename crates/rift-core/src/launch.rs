//! Warp-compatible launch configurations: a named set of tabs, each with a
//! (possibly nested) split layout, working directories and startup commands.
//!
//! Warp stores these as YAML in `~/.warp/launch_configurations/*.yaml`;
//! Rift reads the same schema from `~/.config/rift/launch_configs/*.yaml`
//! (`.yml` too), so Warp's files drop in unchanged.
//!
//! Warp's newer **Tab Configs** are TOML, one tab per file (see
//! [`LaunchConfig::parse_toml`]); Rift reads `*.toml` from the same directory
//! and, read-only, from Warp's own `tab_configs` directories
//! ([`warp_tab_config_dirs`]). Rift writes saved layouts as TOML.
//!
//! ```yaml
//! name: dev-server
//! windows:
//!   - tabs:
//!       - title: server
//!         layout:
//!           cwd: ~/proj/api
//!           commands:
//!             - exec: npm run dev
//!       - title: split
//!         layout:
//!           split_direction: vertical
//!           panes:
//!             - cwd: ~/proj/web
//!             - split_direction: horizontal
//!               panes:
//!                 - cwd: ~/logs
//!                 - cwd: ~
//! active_window_index: 0
//! active_tab_index: 1
//! ```
//!
//! A launch configuration opens in one window, so **every window's tabs are concatenated in
//! document order** into one flat [`LaunchConfig::tabs`] list rather than
//! taking `windows[0]` and discarding the rest. `active_tab` is resolved
//! against that flat list: the tab offset of `active_window_index` plus that
//! window's `active_tab_index` (falling back to the document-level
//! `active_tab_index`), clamped into range.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use crate::profiles::PaneLaunch;

/// One pane, or a split holding more layouts. Recursive, like Warp's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchLayout {
    /// A leaf: one PTY session.
    Pane {
        /// `cwd` with a leading `~` expanded via `$HOME`.
        cwd: Option<PathBuf>,
        /// `commands[].exec`, verbatim and in order.
        commands: Vec<String>,
        /// Warp's `is_focused`.
        focused: bool,
        /// The pane's profile, `shell` (Warp Tab Config) and `args`.
        launch: PaneLaunch,
    },
    /// A split. `vertical` matches Warp's `split_direction: vertical`, i.e.
    /// the panes sit side by side (a vertical divider between them).
    Split {
        vertical: bool,
        panes: Vec<LaunchLayout>,
    },
}

impl LaunchLayout {
    /// A pane with no cwd, no commands and no focus — what a tab with no
    /// `layout` key becomes.
    pub fn default_pane() -> LaunchLayout {
        LaunchLayout::Pane {
            cwd: None,
            commands: Vec::new(),
            focused: false,
            launch: PaneLaunch::default(),
        }
    }

    /// A leaf pane in `cwd` with no startup commands and no focus. A builder
    /// convenience for constructing layouts to export.
    pub fn pane(cwd: Option<PathBuf>) -> LaunchLayout {
        LaunchLayout::Pane {
            cwd,
            commands: Vec::new(),
            focused: false,
            launch: PaneLaunch::default(),
        }
    }

    /// A split of `panes`. `vertical` matches Warp's `split_direction:
    /// vertical` (panes side by side). A builder convenience; callers should
    /// pass at least two panes (a single-pane split collapses on re-parse).
    pub fn split(vertical: bool, panes: Vec<LaunchLayout>) -> LaunchLayout {
        LaunchLayout::Split { vertical, panes }
    }

    /// Every pane's `commands[].exec`, in document order.
    pub fn commands(&self) -> Vec<String> {
        match self {
            LaunchLayout::Pane { commands, .. } => commands.clone(),
            LaunchLayout::Split { panes, .. } => {
                panes.iter().flat_map(LaunchLayout::commands).collect()
            }
        }
    }

    /// Leaves in document order (depth-first).
    pub fn pane_count(&self) -> usize {
        match self {
            LaunchLayout::Pane { .. } => 1,
            LaunchLayout::Split { panes, .. } => panes.iter().map(LaunchLayout::pane_count).sum(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTab {
    pub title: Option<String>,
    /// Warp's tab `color` name (`red`, `green`, ...), verbatim; the app maps
    /// the ones Rift has and ignores the rest.
    pub color: Option<String>,
    pub layout: LaunchLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchConfig {
    pub name: String,
    /// All windows' tabs concatenated (see module docs).
    pub tabs: Vec<LaunchTab>,
    /// Index into `tabs`, always in range.
    pub active_tab: usize,
}

impl LaunchConfig {
    /// Parse one Warp launch-configuration document. `fallback_name` (usually
    /// the file stem) is used when the YAML has no `name`.
    pub fn parse_yaml(fallback_name: &str, src: &str) -> Result<LaunchConfig, String> {
        let doc: RawDoc = serde_yaml_ng::from_str(src).map_err(|e| e.to_string())?;
        let windows = doc.windows.ok_or_else(|| "missing `windows`".to_string())?;

        let mut tabs: Vec<LaunchTab> = Vec::new();
        let mut offsets: Vec<usize> = Vec::with_capacity(windows.len());
        for (wi, w) in windows.iter().enumerate() {
            offsets.push(tabs.len());
            for (ti, t) in w.tabs.iter().enumerate() {
                let layout = match &t.layout {
                    Some(raw) => {
                        convert_layout(raw).map_err(|e| format!("window {wi}, tab {ti}: {e}"))?
                    }
                    None => LaunchLayout::default_pane(),
                };
                tabs.push(LaunchTab {
                    title: t.title.clone(),
                    color: t.color.clone().filter(|c| !c.trim().is_empty()),
                    layout,
                });
            }
        }
        if tabs.is_empty() {
            return Err("launch configuration has no tabs".into());
        }

        let last = tabs.len() - 1;
        let wi = doc.active_window_index.unwrap_or(0).min(windows.len() - 1);
        let local = windows[wi]
            .active_tab_index
            .or(doc.active_tab_index)
            .unwrap_or(0);
        let active_tab = offsets[wi].saturating_add(local).min(last);

        let name = doc
            .name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| fallback_name.to_string());

        Ok(LaunchConfig {
            name,
            tabs,
            active_tab,
        })
    }

    /// Parse one Warp **Tab Config** (TOML, one tab per file):
    /// <https://docs.warp.dev/terminal/windows/tab-configs>.
    ///
    /// `name`, `title`, `color`, a flat `[[panes]]` array whose first entry is
    /// the root, leaves with `directory` / `commands` / `is_focused`, splits
    /// with `split` (`horizontal` = side by side, `vertical` = stacked, the
    /// reverse of the YAML spelling) and `children` (pane ids), and
    /// `[params.<name>]` with a `default` substituted into `{{name}}`. A leaf's
    /// `shell` (Warp) and Rift's `profile` (a `[[profiles]]` name) and `args`
    /// choose what it runs ([`PaneLaunch`]). Other unknown keys (`type`, ...)
    /// are ignored. Input is bounded:
    /// [`MAX_FILE_BYTES`], [`MAX_PANES`], [`MAX_DEPTH`].
    pub fn parse_toml(fallback_name: &str, src: &str) -> Result<LaunchConfig, String> {
        if src.len() > MAX_FILE_BYTES {
            return Err(format!("larger than {MAX_FILE_BYTES} bytes"));
        }
        let doc: TomlDoc = toml::from_str(src).map_err(|e| e.to_string())?;
        if doc.panes.is_empty() {
            return Err("missing `[[panes]]`".into());
        }
        if doc.panes.len() > MAX_PANES {
            return Err(format!("more than {MAX_PANES} panes"));
        }
        let mut by_id: std::collections::HashMap<&str, &TomlPane> =
            std::collections::HashMap::new();
        for p in &doc.panes {
            if by_id.insert(p.id.as_str(), p).is_some() {
                return Err(format!("duplicate pane id {:?}", p.id));
            }
        }
        let params: std::collections::HashMap<&str, &str> = doc
            .params
            .iter()
            .filter_map(|(k, v)| Some((k.as_str(), v.default.as_deref()?)))
            .collect();
        let mut seen = std::collections::HashSet::new();
        let layout = toml_node(&doc.panes[0].id, &by_id, &params, &mut seen, 0)?;
        let title = match doc.title.as_deref() {
            Some(t) => Some(subst(t, &params)?),
            None => None,
        };
        let name = doc
            .name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| fallback_name.to_string());
        Ok(LaunchConfig {
            name,
            tabs: vec![LaunchTab {
                title: title.filter(|t| !t.trim().is_empty()),
                color: doc.color.filter(|c| !c.trim().is_empty()),
                layout,
            }],
            active_tab: 0,
        })
    }

    /// Serialize as a Warp Tab Config (TOML). A tab config holds one tab, so
    /// this errors unless `self` has exactly one; [`save_to_dir`] splits first.
    pub fn to_toml(&self) -> Result<String, String> {
        let [tab] = self.tabs.as_slice() else {
            return Err("a tab config holds exactly one tab".into());
        };
        let mut panes = Vec::new();
        out_toml_node(&tab.layout, &mut panes);
        let doc = OutTomlDoc {
            name: self.name.clone(),
            title: tab.title.clone(),
            color: tab.color.clone(),
            panes,
        };
        toml::to_string(&doc).map_err(|e| e.to_string())
    }

    /// One single-tab config per tab (what [`save_to_dir`] writes). A one-tab
    /// config is returned as is; otherwise tab `i` is named `<name> <i+1>`.
    pub fn split_tabs(&self) -> Vec<LaunchConfig> {
        if self.tabs.len() == 1 {
            return vec![self.clone()];
        }
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| LaunchConfig {
                name: format!("{} {}", self.name, i + 1),
                tabs: vec![t.clone()],
                active_tab: 0,
            })
            .collect()
    }

    /// Every pane in document order (depth-first), useful for the app to
    /// count sessions.
    pub fn pane_count(&self) -> usize {
        self.tabs.iter().map(|t| t.layout.pane_count()).sum()
    }

    /// Serialize back to Warp-compatible launch-configuration YAML — the exact
    /// schema [`LaunchConfig::parse_yaml`] reads. A configuration opens in
    /// one window, so all tabs are written under one `windows` entry, and `active_tab` is
    /// emitted as the document-level `active_tab_index`.
    ///
    /// `cwd` paths are written as-is (they are already `$HOME`-expanded absolute
    /// paths after parsing), so `to_yaml` → `parse_yaml` round-trips to an equal
    /// [`LaunchConfig`].
    pub fn to_yaml(&self) -> String {
        let doc = OutDoc {
            name: self.name.clone(),
            windows: vec![OutWindow {
                tabs: self
                    .tabs
                    .iter()
                    .map(|t| OutTab {
                        title: t.title.clone(),
                        color: t.color.clone(),
                        layout: out_layout(&t.layout),
                    })
                    .collect(),
            }],
            active_tab_index: self.active_tab,
        };
        // Our own owned, non-cyclic data always serializes.
        serde_yaml_ng::to_string(&doc).unwrap_or_default()
    }
}

/// `~/.config/rift/launch_configs/<slug>.toml` (Warp Tab Config format), never
/// overwriting: if the name's slug is taken, `-2`, `-3`, … is appended. A
/// multi-tab config is written as one file per tab ([`LaunchConfig::split_tabs`]).
/// Returns the paths written. Errors if [`launch_dir`] is unavailable (no
/// config directory).
pub fn save(cfg: &LaunchConfig) -> std::io::Result<Vec<PathBuf>> {
    let dir = launch_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory for launch configs",
        )
    })?;
    save_to_dir(cfg, &dir)
}

/// [`save`] against an explicit directory (also the unit-test entry point).
/// Creates `dir` if needed, writes `<slug>.toml` without ever overwriting an
/// existing file (`-2`, `-3`, … suffixes), and returns the paths written.
pub fn save_to_dir(cfg: &LaunchConfig, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    for tab_cfg in cfg.split_tabs() {
        let body = tab_cfg
            .to_toml()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let slug = slugify(&tab_cfg.name);
        let mut n = 2u32;
        let mut path = dir.join(format!("{slug}.toml"));
        // `create_new` makes the never-overwrite check atomic.
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    std::io::Write::write_all(&mut f, body.as_bytes())?;
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    path = dir.join(format!("{slug}-{n}.toml"));
                    n += 1;
                }
                Err(e) => return Err(e),
            }
        }
        written.push(path);
    }
    Ok(written)
}

/// A filesystem-safe file stem: ASCII alphanumerics lowercased, every other run
/// of characters collapsed to a single `-`, no leading/trailing `-`. Empty
/// names (or names with no usable characters) become `launch`.
fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() {
        out.push_str("launch");
    }
    out
}

fn out_layout(l: &LaunchLayout) -> OutLayout {
    match l {
        LaunchLayout::Pane {
            cwd,
            commands,
            focused,
            launch,
        } => OutLayout {
            cwd: cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
            commands: commands
                .iter()
                .map(|exec| OutCommand { exec: exec.clone() })
                .collect(),
            is_focused: *focused,
            launch: launch.clone(),
            split_direction: None,
            panes: None,
        },
        LaunchLayout::Split { vertical, panes } => OutLayout {
            cwd: None,
            commands: Vec::new(),
            is_focused: false,
            launch: PaneLaunch::default(),
            split_direction: Some(if *vertical { "vertical" } else { "horizontal" }.to_string()),
            panes: Some(panes.iter().map(out_layout).collect()),
        },
    }
}

fn convert_layout(raw: &RawLayout) -> Result<LaunchLayout, String> {
    // `split_direction` is validated wherever it appears, so a typo is an
    // error rather than a silently-wrong orientation.
    let vertical = match raw.split_direction.as_deref().map(str::trim) {
        None => true,
        Some("vertical") => true,
        Some("horizontal") => false,
        Some(other) => return Err(format!("unknown split_direction {other:?}")),
    };

    let Some(panes) = raw.panes.as_ref() else {
        // Leaf.
        return Ok(LaunchLayout::Pane {
            cwd: raw
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(expand_tilde),
            commands: raw.commands.iter().map(|c| c.exec.clone()).collect(),
            focused: raw.is_focused.unwrap_or(false),
            launch: raw.launch.clone(),
        });
    };

    if panes.is_empty() {
        return Err("split has no panes".into());
    }
    if panes.len() == 1 {
        // A one-element `panes` list is not a split; collapse it.
        return convert_layout(&panes[0]);
    }
    let panes = panes
        .iter()
        .map(convert_layout)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LaunchLayout::Split { vertical, panes })
}

/// `~` / `~/x` via `$HOME`; anything else (including `~user`) is left alone.
pub(crate) fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(s)
}

/// `~/.config/rift/launch_configs`.
pub fn launch_dir() -> Option<PathBuf> {
    crate::config::config_dir().map(|d| d.join("launch_configs"))
}

/// Warp's own Tab Config directories that exist on this machine (stable and
/// preview): `~/.warp/tab_configs` and `~/.warp-preview/tab_configs` on macOS,
/// `$XDG_DATA_HOME/warp-terminal[-preview]/tab_configs` on Linux. Read-only:
/// Rift never writes there.
pub fn warp_tab_config_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            dirs.push(h.join(".warp/tab_configs"));
            dirs.push(h.join(".warp-preview/tab_configs"));
        }
    } else {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.map(|h| h.join(".local/share")));
        if let Some(d) = data {
            dirs.push(d.join("warp-terminal/tab_configs"));
            dirs.push(d.join("warp-terminal-preview/tab_configs"));
        }
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// Every `*.yaml` / `*.yml` / `*.toml` in [`launch_dir`], then (read-only)
/// the `*.toml` Tab Configs in [`warp_tab_config_dirs`], sorted by name. A
/// Warp config whose name matches one of Rift's own is shadowed by it. Files
/// that fail to parse are logged and skipped.
pub fn load_all() -> Vec<LaunchConfig> {
    load_all_reporting().0
}

/// A config file [`load_all`] skipped: its file name and why, on one line.
#[derive(Debug, Clone, PartialEq)]
pub struct Skipped {
    pub file: String,
    pub reason: String,
}

/// [`load_all`], also naming the files it skipped (so the palette can say
/// why a config is missing).
pub fn load_all_reporting() -> (Vec<LaunchConfig>, Vec<Skipped>) {
    let (mut out, mut skipped) = match launch_dir() {
        Some(dir) => load_from_dir_reporting(&dir),
        None => (Vec::new(), Vec::new()),
    };
    for dir in warp_tab_config_dirs() {
        let (configs, bad) = load_from_dir_reporting(&dir);
        skipped.extend(bad);
        for c in configs {
            if !out.iter().any(|o| o.name.eq_ignore_ascii_case(&c.name)) {
                out.push(c);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, skipped)
}

/// [`load_all`] against one explicit directory (also the unit-test entry
/// point, so tests never have to move `$XDG_CONFIG_HOME`).
pub fn load_from_dir(dir: &Path) -> Vec<LaunchConfig> {
    load_from_dir_reporting(dir).0
}

/// [`load_from_dir`], also naming the files it skipped.
pub fn load_from_dir_reporting(dir: &Path) -> (Vec<LaunchConfig>, Vec<Skipped>) {
    let mut skipped = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (Vec::new(), skipped);
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    let mut out: Vec<LaunchConfig> = Vec::new();
    for p in paths {
        let ext = p.extension().and_then(|s| s.to_str());
        if !matches!(ext, Some("yaml") | Some("yml") | Some("toml")) {
            continue;
        }
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("launch")
            .to_string();
        let file = p
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut skip = |reason: String| {
            log::warn!("launch config {p:?}: {reason}");
            // one short line: toml's errors quote the source over several
            let line = reason.lines().next().unwrap_or_default().trim();
            let mut reason: String = line.chars().take(80).collect();
            if line.chars().count() > 80 {
                reason.push('…');
            }
            skipped.push(Skipped {
                file: file.clone(),
                reason,
            });
        };
        if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES as u64 {
            skip(format!("larger than {MAX_FILE_BYTES} bytes"));
            continue;
        }
        match std::fs::read_to_string(&p) {
            Ok(body) => {
                let parsed = if ext == Some("toml") {
                    LaunchConfig::parse_toml(&stem, &body)
                } else {
                    LaunchConfig::parse_yaml(&stem, &body)
                };
                match parsed {
                    Ok(c) => out.push(c),
                    Err(err) => skip(err),
                }
            }
            Err(err) => skip(err.to_string()),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, skipped)
}

/// Largest config file read, in bytes.
pub const MAX_FILE_BYTES: usize = 256 * 1024;
/// Most panes one tab config may declare.
pub const MAX_PANES: usize = 64;
/// Deepest split nesting read from a tab config.
pub const MAX_DEPTH: usize = 16;

// ---- Warp Tab Config (TOML) -----------------------------------------------

#[derive(Debug, Deserialize)]
struct TomlDoc {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    panes: Vec<TomlPane>,
    #[serde(default)]
    params: std::collections::BTreeMap<String, TomlParam>,
}

#[derive(Debug, Deserialize)]
struct TomlParam {
    #[serde(default)]
    default: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TomlPane {
    id: String,
    #[serde(default)]
    directory: Option<String>,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    is_focused: Option<bool>,
    #[serde(default)]
    split: Option<String>,
    #[serde(default)]
    children: Vec<String>,
    /// `shell` (Warp), `profile` and `args` (Rift).
    #[serde(flatten)]
    launch: PaneLaunch,
}

/// Replace `{{param}}` with its default. A placeholder with no default (or a
/// reserved one such as `autogenerated_branch_name`) is an error: Rift cannot
/// prompt for it, and running a command with a literal `{{x}}` is worse.
fn subst(s: &str, params: &std::collections::HashMap<&str, &str>) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find("}}") else {
            out.push_str(&rest[i..]);
            return Ok(out);
        };
        let key = after[..j].trim();
        match params.get(key) {
            Some(v) => out.push_str(v),
            None => return Err(format!("parameter {key:?} has no default")),
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn toml_node(
    id: &str,
    by_id: &std::collections::HashMap<&str, &TomlPane>,
    params: &std::collections::HashMap<&str, &str>,
    seen: &mut std::collections::HashSet<String>,
    depth: usize,
) -> Result<LaunchLayout, String> {
    if depth > MAX_DEPTH {
        return Err(format!("splits nested deeper than {MAX_DEPTH}"));
    }
    let pane = by_id
        .get(id)
        .ok_or_else(|| format!("unknown pane id {id:?}"))?;
    if !seen.insert(id.to_string()) {
        return Err(format!("pane {id:?} is used twice"));
    }
    let Some(split) = pane.split.as_deref().map(str::trim) else {
        let cwd = match pane.directory.as_deref().map(str::trim) {
            Some(d) if !d.is_empty() => Some(expand_tilde(&subst(d, params)?)),
            _ => None,
        };
        return Ok(LaunchLayout::Pane {
            cwd,
            commands: pane
                .commands
                .iter()
                .map(|c| subst(c, params))
                .collect::<Result<_, _>>()?,
            focused: pane.is_focused.unwrap_or(false),
            launch: PaneLaunch {
                profile: pane
                    .launch
                    .profile
                    .as_deref()
                    .map(|p| subst(p, params))
                    .transpose()?,
                shell: pane
                    .launch
                    .shell
                    .as_deref()
                    .map(|s| subst(s, params))
                    .transpose()?,
                args: pane
                    .launch
                    .args
                    .iter()
                    .map(|a| subst(a, params))
                    .collect::<Result<_, _>>()?,
            },
        });
    };
    // Warp's TOML spelling: `horizontal` is left-to-right, `vertical` stacked.
    let vertical = match split {
        "horizontal" => true,
        "vertical" => false,
        other => return Err(format!("unknown split {other:?}")),
    };
    if pane.children.len() < 2 {
        return Err(format!("split {id:?} needs at least two children"));
    }
    let panes = pane
        .children
        .iter()
        .map(|c| toml_node(c, by_id, params, seen, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LaunchLayout::Split { vertical, panes })
}

#[derive(Debug, Serialize)]
struct OutTomlDoc {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    panes: Vec<OutTomlPane>,
}

#[derive(Debug, Serialize)]
struct OutTomlPane {
    id: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    directory: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    commands: Vec<String>,
    #[serde(skip_serializing_if = "is_false")]
    is_focused: bool,
    #[serde(flatten)]
    launch: PaneLaunch,
    #[serde(skip_serializing_if = "Option::is_none")]
    split: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<String>,
}

/// Append `l` (root first, then descendants, depth-first) to `out`; returns
/// its pane id.
fn out_toml_node(l: &LaunchLayout, out: &mut Vec<OutTomlPane>) -> String {
    let id = format!("pane_{}", out.len() + 1);
    let slot = out.len();
    match l {
        LaunchLayout::Pane {
            cwd,
            commands,
            focused,
            launch,
        } => out.push(OutTomlPane {
            id: id.clone(),
            kind: Some("terminal"),
            directory: cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
            commands: commands.clone(),
            is_focused: *focused,
            launch: launch.clone(),
            split: None,
            children: Vec::new(),
        }),
        LaunchLayout::Split { vertical, panes } => {
            out.push(OutTomlPane {
                id: id.clone(),
                kind: None,
                directory: None,
                commands: Vec::new(),
                is_focused: false,
                launch: PaneLaunch::default(),
                split: Some(if *vertical { "horizontal" } else { "vertical" }),
                children: Vec::new(),
            });
            let kids = panes.iter().map(|p| out_toml_node(p, out)).collect();
            out[slot].children = kids;
        }
    }
    id
}

// ---- wire format: writing ----------------------------------------------
//
// A separate set of structs from the reading `Raw*` ones so the emitted shape
// is fully under our control: absent keys are skipped rather than written as
// null/empty, which is what keeps `to_yaml` → `parse_yaml` an identity.

#[derive(Debug, Serialize)]
struct OutDoc {
    name: String,
    windows: Vec<OutWindow>,
    active_tab_index: usize,
}

#[derive(Debug, Serialize)]
struct OutWindow {
    tabs: Vec<OutTab>,
}

#[derive(Debug, Serialize)]
struct OutTab {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    layout: OutLayout,
}

#[derive(Debug, Serialize)]
struct OutLayout {
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    commands: Vec<OutCommand>,
    #[serde(skip_serializing_if = "is_false")]
    is_focused: bool,
    #[serde(flatten)]
    launch: PaneLaunch,
    #[serde(skip_serializing_if = "Option::is_none")]
    split_direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    panes: Option<Vec<OutLayout>>,
}

#[derive(Debug, Serialize)]
struct OutCommand {
    exec: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

// ---- wire format: reading ----------------------------------------------

#[derive(Debug, Deserialize)]
struct RawDoc {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    windows: Option<Vec<RawWindow>>,
    #[serde(default)]
    active_window_index: Option<usize>,
    #[serde(default)]
    active_tab_index: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RawWindow {
    #[serde(default)]
    tabs: Vec<RawTab>,
    #[serde(default)]
    active_tab_index: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RawTab {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    layout: Option<RawLayout>,
    #[serde(default)]
    color: Option<String>,
    // Any other Warp-only keys are ignored.
}

#[derive(Debug, Deserialize)]
struct RawLayout {
    /// `deserialize_with` only runs when the key is present, so a missing
    /// `cwd` stays `None` while an explicit `cwd: ~` (YAML null, which is how
    /// Warp spells the home directory) survives as `~`.
    #[serde(default, deserialize_with = "de_cwd")]
    cwd: Option<String>,
    #[serde(default)]
    commands: Vec<RawCommand>,
    #[serde(default)]
    is_focused: Option<bool>,
    #[serde(default)]
    split_direction: Option<String>,
    #[serde(default)]
    panes: Option<Vec<RawLayout>>,
    /// `shell`, `profile` and `args`, as in Tab Configs.
    #[serde(flatten)]
    launch: PaneLaunch,
}

#[derive(Debug, Deserialize)]
struct RawCommand {
    exec: String,
}

fn de_cwd<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    match serde_yaml_ng::Value::deserialize(d)? {
        // `cwd: ~` is YAML null; in Warp files it means the home directory.
        serde_yaml_ng::Value::Null => Ok(Some("~".into())),
        serde_yaml_ng::Value::String(s) => Ok(Some(s)),
        other => Err(D::Error::custom(format!(
            "cwd must be a string, got {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
name: dev-server
windows:
  - tabs:
      - title: server
        layout:
          cwd: ~/proj/api
          commands:
            - exec: npm run dev
      - title: split
        layout:
          split_direction: vertical
          panes:
            - cwd: ~/proj/web
              commands:
                - exec: pnpm dev
            - split_direction: horizontal
              panes:
                - cwd: ~/logs
                - cwd: ~
        color: blue
active_window_index: 0
active_tab_index: 1
"#;

    fn home() -> PathBuf {
        PathBuf::from(std::env::var_os("HOME").expect("HOME"))
    }

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("rift-launch-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn full_warp_example_parses() {
        let c = LaunchConfig::parse_yaml("file-stem", FULL).unwrap();
        assert_eq!(c.name, "dev-server");
        assert_eq!(c.tabs.len(), 2);
        assert_eq!(c.active_tab, 1);
        assert_eq!(c.pane_count(), 4);

        // Tab 0: a leaf with cwd + one command.
        assert_eq!(c.tabs[0].title.as_deref(), Some("server"));
        let LaunchLayout::Pane {
            cwd,
            commands,
            focused,
            ..
        } = &c.tabs[0].layout
        else {
            panic!("tab 0 should be a leaf pane");
        };
        assert_eq!(cwd.as_deref(), Some(home().join("proj/api").as_path()));
        assert_eq!(commands, &vec!["npm run dev".to_string()]);
        assert!(!focused);

        // Tab 1: vertical split, second pane a horizontal split of two leaves.
        assert_eq!(c.tabs[1].title.as_deref(), Some("split"));
        let LaunchLayout::Split { vertical, panes } = &c.tabs[1].layout else {
            panic!("tab 1 should be a split");
        };
        assert!(vertical);
        assert_eq!(panes.len(), 2);
        assert_eq!(
            panes[0],
            LaunchLayout::Pane {
                cwd: Some(home().join("proj/web")),
                commands: vec!["pnpm dev".to_string()],
                focused: false,
                launch: PaneLaunch::default(),
            }
        );
        assert_eq!(
            panes[1],
            LaunchLayout::Split {
                vertical: false,
                panes: vec![
                    LaunchLayout::Pane {
                        cwd: Some(home().join("logs")),
                        commands: vec![],
                        focused: false,
                        launch: PaneLaunch::default(),
                    },
                    LaunchLayout::Pane {
                        cwd: Some(home()),
                        commands: vec![],
                        focused: false,
                        launch: PaneLaunch::default(),
                    },
                ],
            }
        );
    }

    #[test]
    fn minimal_name_and_one_tab() {
        let c = LaunchConfig::parse_yaml("stem", "name: mini\nwindows:\n  - tabs:\n      - {}\n")
            .unwrap();
        assert_eq!(c.name, "mini");
        assert_eq!(c.tabs.len(), 1);
        assert_eq!(c.tabs[0].title, None);
        assert_eq!(c.tabs[0].layout, LaunchLayout::default_pane());
        assert_eq!(c.pane_count(), 1);
        assert_eq!(c.active_tab, 0);
    }

    #[test]
    fn name_falls_back_to_file_stem() {
        let c = LaunchConfig::parse_yaml("my-stem", "windows:\n  - tabs:\n      - {}\n").unwrap();
        assert_eq!(c.name, "my-stem");
    }

    #[test]
    fn missing_windows_is_error() {
        let err = LaunchConfig::parse_yaml("stem", "name: nope\n").unwrap_err();
        assert!(err.contains("windows"), "{err}");
        // Present-but-empty is an error too: nothing to launch.
        assert!(LaunchConfig::parse_yaml("stem", "name: nope\nwindows: []\n").is_err());
    }

    #[test]
    fn unknown_split_direction_is_error() {
        let src = "windows:\n  - tabs:\n      - layout:\n          split_direction: diagonal\n          panes:\n            - cwd: /a\n            - cwd: /b\n";
        let err = LaunchConfig::parse_yaml("stem", src).unwrap_err();
        assert!(err.contains("diagonal"), "{err}");
        // ...and the good ones still parse.
        for dir in ["vertical", "horizontal"] {
            let ok = src.replace("diagonal", dir);
            assert!(LaunchConfig::parse_yaml("stem", &ok).is_ok(), "{dir}");
        }
    }

    #[test]
    fn empty_panes_is_error() {
        let err = LaunchConfig::parse_yaml(
            "stem",
            "windows:\n  - tabs:\n      - layout:\n          split_direction: vertical\n          panes: []\n",
        )
        .unwrap_err();
        assert!(err.contains("no panes"), "{err}");
    }

    #[test]
    fn single_element_panes_collapses() {
        let c = LaunchConfig::parse_yaml(
            "stem",
            "windows:\n  - tabs:\n      - layout:\n          split_direction: horizontal\n          panes:\n            - cwd: /only\n              is_focused: true\n",
        )
        .unwrap();
        assert_eq!(
            c.tabs[0].layout,
            LaunchLayout::Pane {
                cwd: Some(PathBuf::from("/only")),
                commands: vec![],
                focused: true,
                launch: PaneLaunch::default(),
            }
        );
        assert_eq!(c.pane_count(), 1);
    }

    #[test]
    fn split_direction_defaults_to_vertical() {
        let c = LaunchConfig::parse_yaml(
            "stem",
            "windows:\n  - tabs:\n      - layout:\n          panes:\n            - cwd: /a\n            - cwd: /b\n",
        )
        .unwrap();
        assert!(matches!(
            c.tabs[0].layout,
            LaunchLayout::Split { vertical: true, .. }
        ));
        assert_eq!(c.pane_count(), 2);
    }

    #[test]
    fn windows_are_concatenated_and_active_tab_offset() {
        let src = "windows:\n  - tabs:\n      - title: a\n      - title: b\n  - tabs:\n      - title: c\n    active_tab_index: 0\nactive_window_index: 1\n";
        let c = LaunchConfig::parse_yaml("stem", src).unwrap();
        let titles: Vec<_> = c.tabs.iter().map(|t| t.title.clone().unwrap()).collect();
        assert_eq!(titles, ["a", "b", "c"]);
        assert_eq!(c.active_tab, 2);
    }

    #[test]
    fn out_of_range_active_indices_are_clamped() {
        let c = LaunchConfig::parse_yaml(
            "stem",
            "windows:\n  - tabs:\n      - {}\n      - {}\nactive_window_index: 9\nactive_tab_index: 9\n",
        )
        .unwrap();
        assert_eq!(c.active_tab, 1);
    }

    #[test]
    fn load_from_dir_reads_yaml_and_yml_sorted_and_skips_bad() {
        let dir = scratch("load");
        std::fs::write(
            dir.join("beta.yaml"),
            "name: beta\nwindows:\n  - tabs:\n      - {}\n",
        )
        .unwrap();
        // No `name:` — takes the file stem, which also sorts it first.
        std::fs::write(
            dir.join("alpha.yml"),
            "windows:\n  - tabs:\n      - {}\n      - {}\n",
        )
        .unwrap();
        std::fs::write(dir.join("broken.yaml"), "name: broken\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "name: ignored\nwindows: []\n").unwrap();

        let all = load_from_dir(&dir);
        let names: Vec<_> = all.iter().map(|c| c.name.clone()).collect();
        assert_eq!(names, ["alpha", "beta"]);
        assert_eq!(all[0].pane_count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn skipped_files_are_named_with_a_one_line_reason() {
        let dir = scratch("skipped");
        std::fs::write(
            dir.join("ok.toml"),
            "name = \"ok\"\n[[panes]]\nid = \"a\"\ntype = \"terminal\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("param.toml"),
            "title = \"{{branch}}\"\n[params.branch]\n[[panes]]\nid = \"a\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("bad.toml"), "[[panes]\nid = \n").unwrap();
        let (configs, skipped) = load_from_dir_reporting(&dir);
        assert_eq!(configs.len(), 1);
        let files: Vec<_> = skipped.iter().map(|s| s.file.as_str()).collect();
        assert_eq!(files, ["bad.toml", "param.toml"]);
        assert_eq!(skipped[1].reason, "parameter \"branch\" has no default");
        assert!(
            skipped
                .iter()
                .all(|s| !s.reason.contains('\n') && !s.reason.is_empty())
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn absent_cwd_is_none_but_yaml_null_cwd_is_home() {
        let c = LaunchConfig::parse_yaml(
            "stem",
            "windows:\n  - tabs:\n      - layout:\n          commands:\n            - exec: htop\n      - layout:\n          cwd: ~\n",
        )
        .unwrap();
        assert_eq!(
            c.tabs[0].layout,
            LaunchLayout::Pane {
                cwd: None,
                commands: vec!["htop".to_string()],
                focused: false,
                launch: PaneLaunch::default(),
            }
        );
        assert_eq!(
            c.tabs[1].layout,
            LaunchLayout::Pane {
                cwd: Some(home()),
                commands: vec![],
                focused: false,
                launch: PaneLaunch::default(),
            }
        );
    }

    #[test]
    fn load_from_missing_dir_is_empty() {
        assert!(load_from_dir(Path::new("/nonexistent/rift/launch_configs")).is_empty());
    }

    #[test]
    fn launch_dir_is_under_config_dir() {
        if let Some(d) = launch_dir() {
            assert_eq!(
                d,
                crate::config::config_dir().unwrap().join("launch_configs")
            );
        }
    }

    #[test]
    fn to_yaml_roundtrips_the_full_example() {
        let orig = LaunchConfig::parse_yaml("file-stem", FULL).unwrap();
        let yaml = orig.to_yaml();
        let back = LaunchConfig::parse_yaml("other-stem", &yaml).unwrap();
        // Everything survives: name, nested split layout, cwds, commands,
        // titles and the active tab index.
        assert_eq!(back, orig, "emitted yaml was:\n{yaml}");
    }

    #[test]
    fn to_yaml_roundtrips_builder_layout() {
        let cfg = LaunchConfig {
            name: "My Config".into(),
            tabs: vec![
                LaunchTab {
                    title: Some("editor".into()),
                    color: Some("red".into()),
                    layout: LaunchLayout::Pane {
                        cwd: Some(PathBuf::from("/work/app")),
                        commands: vec!["nvim".into()],
                        focused: true,
                        launch: PaneLaunch::default(),
                    },
                },
                LaunchTab {
                    title: None,
                    color: None,
                    layout: LaunchLayout::split(
                        false,
                        vec![
                            LaunchLayout::pane(Some(PathBuf::from("/work/web"))),
                            LaunchLayout::split(
                                true,
                                vec![
                                    LaunchLayout::pane(Some(PathBuf::from("/var/log"))),
                                    LaunchLayout::pane(None),
                                ],
                            ),
                        ],
                    ),
                },
            ],
            active_tab: 1,
        };

        let yaml = cfg.to_yaml();
        let back = LaunchConfig::parse_yaml("stem", &yaml).unwrap();
        assert_eq!(back, cfg, "emitted yaml was:\n{yaml}");
    }

    #[test]
    fn save_to_dir_writes_toml_and_suffixes_without_overwriting() {
        let dir = scratch("save");
        let cfg = LaunchConfig::parse_yaml("stem", FULL).unwrap();
        // FULL has two tabs: one file each, named by tab.
        let p1 = save_to_dir(&cfg, &dir).unwrap();
        let names: Vec<_> = p1
            .iter()
            .map(|p| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["dev-server-1.toml", "dev-server-2.toml"]);

        // Same name again -> suffixed, originals untouched.
        let p2 = save_to_dir(&cfg, &dir).unwrap();
        assert_eq!(p2[0].file_name().unwrap(), "dev-server-1-2.toml");
        let first = std::fs::read_to_string(&p1[0]).unwrap();
        save_to_dir(&cfg, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(&p1[0]).unwrap(), first);

        // The written files parse back to the split tabs.
        let back = LaunchConfig::parse_toml("x", &first).unwrap();
        assert_eq!(back, cfg.split_tabs()[0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/tab_configs")
            .join(name);
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn toml_single_pane_fixture_parses() {
        let c = LaunchConfig::parse_toml("stem", &fixture("dev_server.toml")).unwrap();
        assert_eq!(c.name, "Dev Server");
        assert_eq!(c.tabs.len(), 1);
        assert_eq!(c.tabs[0].title.as_deref(), Some("api"));
        assert_eq!(c.tabs[0].color.as_deref(), Some("blue"));
        assert_eq!(
            c.tabs[0].layout,
            LaunchLayout::Pane {
                cwd: Some(home().join("code/api")),
                commands: vec!["npm install".into(), "npm run dev".into()],
                focused: true,
                // Warp's per-pane shell is honoured, not ignored
                launch: PaneLaunch {
                    shell: Some("zsh".into()),
                    ..Default::default()
                },
            }
        );
    }

    #[test]
    fn pane_shell_profile_and_args_roundtrip_with_spaces_and_quotes() {
        let odd = |s: &str| format!("/tmp/it's a \"{s}\" dir");
        let launch = PaneLaunch {
            profile: Some("dev box".into()),
            shell: Some("/opt/my shells/fish".into()),
            args: vec!["-C".into(), "echo \"a b\" 'c'".into()],
        };
        let cfg = LaunchConfig {
            name: "odd".into(),
            tabs: vec![LaunchTab {
                title: None,
                color: None,
                layout: LaunchLayout::split(
                    true,
                    vec![
                        LaunchLayout::Pane {
                            cwd: Some(PathBuf::from(odd("one"))),
                            commands: vec![],
                            focused: false,
                            launch: launch.clone(),
                        },
                        LaunchLayout::Pane {
                            cwd: Some(PathBuf::from(odd("two"))),
                            commands: vec![],
                            focused: true,
                            launch: PaneLaunch::profile("zsh"),
                        },
                    ],
                ),
            }],
            active_tab: 0,
        };
        let toml = cfg.to_toml().unwrap();
        assert_eq!(
            LaunchConfig::parse_toml("odd", &toml).unwrap(),
            cfg,
            "{toml}"
        );
        let yaml = cfg.to_yaml();
        assert_eq!(
            LaunchConfig::parse_yaml("odd", &yaml).unwrap(),
            cfg,
            "{yaml}"
        );
        // a pane without any keeps the old shape: no launch keys written
        let plain = LaunchConfig {
            tabs: vec![LaunchTab {
                title: None,
                color: None,
                layout: LaunchLayout::pane(Some("/w".into())),
            }],
            ..cfg
        };
        let out = plain.to_toml().unwrap();
        assert!(!out.contains("shell") && !out.contains("profile") && !out.contains("args"));
    }

    #[test]
    fn toml_params_reach_pane_shell_profile_and_args() {
        let src = r#"
name = "p"
[params.who]
default = "dev"
[[panes]]
id = "a"
profile = "{{who}}"
shell = "/bin/{{who}}sh"
args = ["--name", "{{who}} x"]
"#;
        let c = LaunchConfig::parse_toml("p", src).unwrap();
        let LaunchLayout::Pane { launch, .. } = &c.tabs[0].layout else {
            panic!("leaf expected");
        };
        assert_eq!(
            launch,
            &PaneLaunch {
                profile: Some("dev".into()),
                shell: Some("/bin/devsh".into()),
                args: vec!["--name".into(), "dev x".into()],
            }
        );
        // a placeholder without default is an error, as for commands
        assert!(LaunchConfig::parse_toml("p", &src.replace("default = \"dev\"", "")).is_err());
    }

    #[test]
    fn toml_split_fixture_maps_directions_and_ignores_unknown_keys() {
        let c = LaunchConfig::parse_toml("stem", &fixture("server_plus_logs.toml")).unwrap();
        assert_eq!(c.pane_count(), 3);
        // TOML `horizontal` = left-to-right = Rift `vertical: true`.
        let LaunchLayout::Split { vertical, panes } = &c.tabs[0].layout else {
            panic!("root should be a split");
        };
        assert!(vertical);
        assert!(matches!(
            panes[1],
            LaunchLayout::Split {
                vertical: false,
                ..
            }
        ));
        assert!(matches!(panes[0], LaunchLayout::Pane { focused: true, .. }));
    }

    #[test]
    fn toml_params_use_defaults_and_missing_default_is_error() {
        let c = LaunchConfig::parse_toml("stem", &fixture("worktree.toml")).unwrap();
        assert_eq!(c.tabs[0].title.as_deref(), Some("main"));
        let LaunchLayout::Pane { commands, .. } = &c.tabs[0].layout else {
            panic!("leaf expected");
        };
        assert_eq!(commands, &vec!["git switch main".to_string()]);
        let no_default = fixture("worktree.toml").replace("default = \"main\"", "");
        let err = LaunchConfig::parse_toml("stem", &no_default).unwrap_err();
        assert!(err.contains("branch"), "{err}");
    }

    #[test]
    fn toml_rejects_bad_trees() {
        let leaf = "[[panes]]\nid = \"a\"\n";
        for (src, want) in [
            ("name = \"x\"\n".to_string(), "panes"),
            (format!("{leaf}{leaf}"), "duplicate"),
            ("[[panes]]\nid = \"r\"\nsplit = \"horizontal\"\nchildren = [\"r\", \"r\"]\n".into(), "twice"),
            ("[[panes]]\nid = \"r\"\nsplit = \"horizontal\"\nchildren = [\"a\"]\n".into(), "two children"),
            ("[[panes]]\nid = \"r\"\nsplit = \"horizontal\"\nchildren = [\"a\", \"nope\"]\n[[panes]]\nid = \"a\"\n".into(), "nope"),
            ("[[panes]]\nid = \"r\"\nsplit = \"diagonal\"\nchildren = [\"a\", \"b\"]\n".into(), "diagonal"),
        ] {
            let err = LaunchConfig::parse_toml("stem", &src).unwrap_err();
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    #[test]
    fn toml_bounds_depth_panes_and_size() {
        // a chain deeper than MAX_DEPTH
        let mut src = String::new();
        for i in 0..=MAX_DEPTH + 1 {
            src.push_str(&format!(
                "[[panes]]\nid = \"s{i}\"\nsplit = \"horizontal\"\nchildren = [\"s{}\", \"leaf\"]\n",
                i + 1
            ));
        }
        src.push_str("[[panes]]\nid = \"leaf\"\n");
        let err = LaunchConfig::parse_toml("stem", &src).unwrap_err();
        assert!(err.contains("deeper"), "{err}");

        let many: String = (0..=MAX_PANES)
            .map(|i| format!("[[panes]]\nid = \"p{i}\"\n"))
            .collect();
        assert!(
            LaunchConfig::parse_toml("stem", &many)
                .unwrap_err()
                .contains("more than")
        );

        let big = format!(
            "name = \"{}\"\n{}",
            "x".repeat(MAX_FILE_BYTES),
            "[[panes]]\nid = \"a\"\n"
        );
        assert!(
            LaunchConfig::parse_toml("stem", &big)
                .unwrap_err()
                .contains("larger")
        );
    }

    #[test]
    fn to_toml_roundtrips_nested_layout() {
        let cfg = LaunchConfig {
            name: "Round Trip".into(),
            tabs: vec![LaunchTab {
                title: Some("t".into()),
                color: Some("cyan".into()),
                layout: LaunchLayout::split(
                    true,
                    vec![
                        LaunchLayout::Pane {
                            cwd: Some(PathBuf::from("/work/app")),
                            commands: vec!["nvim".into(), "echo \"hi\"".into()],
                            focused: true,
                            launch: PaneLaunch::default(),
                        },
                        LaunchLayout::split(
                            false,
                            vec![
                                LaunchLayout::pane(Some(PathBuf::from("/var/log"))),
                                LaunchLayout::pane(None),
                            ],
                        ),
                    ],
                ),
            }],
            active_tab: 0,
        };
        let toml = cfg.to_toml().unwrap();
        assert_eq!(
            LaunchConfig::parse_toml("stem", &toml).unwrap(),
            cfg,
            "{toml}"
        );
        // multi-tab configs are not a tab config
        let mut two = cfg.clone();
        two.tabs.push(two.tabs[0].clone());
        assert!(two.to_toml().is_err());
    }

    #[test]
    fn save_then_load_roundtrips_and_loads_yaml_and_toml_together() {
        let dir = scratch("mixed");
        let cfg = LaunchConfig::parse_toml("stem", &fixture("server_plus_logs.toml")).unwrap();
        let written = save_to_dir(&cfg, &dir).unwrap();
        assert_eq!(written[0].file_name().unwrap(), "server-plus-logs.toml");
        std::fs::write(
            dir.join("old.yaml"),
            "name: old\nwindows:\n  - tabs:\n      - {}\n",
        )
        .unwrap();
        std::fs::write(dir.join("broken.toml"), "name = [\n").unwrap();
        std::fs::write(dir.join("empty.toml"), "name = \"no panes\"\n").unwrap();
        let all = load_from_dir(&dir);
        let names: Vec<_> = all.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Server Plus Logs", "old"]);
        assert_eq!(all[0], cfg);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn yaml_tab_color_is_read() {
        let c = LaunchConfig::parse_yaml(
            "stem",
            "windows:\n  - tabs:\n      - color: green\n        layout:\n          cwd: /a\n",
        )
        .unwrap();
        assert_eq!(c.tabs[0].color.as_deref(), Some("green"));
    }

    #[test]
    fn slugify_handles_messy_names() {
        assert_eq!(slugify("Dev Server"), "dev-server");
        assert_eq!(slugify("  a/b  c "), "a-b-c");
        assert_eq!(slugify("!!!"), "launch");
        assert_eq!(slugify(""), "launch");
        assert_eq!(slugify("Already-Slugged_2"), "already-slugged-2");
    }
}
