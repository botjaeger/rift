//! Warp-compatible launch configurations: a named set of tabs, each with a
//! (possibly nested) split layout, working directories and startup commands.
//!
//! Warp stores these as YAML in `~/.warp/launch_configurations/*.yaml`;
//! Rift reads the same schema from `~/.config/rift/launch_configs/*.yaml`
//! (`.yml` too), so Warp's files drop in unchanged.
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
//! Rift has a single window, so **every window's tabs are concatenated in
//! document order** into one flat [`LaunchConfig::tabs`] list rather than
//! taking `windows[0]` and discarding the rest. `active_tab` is resolved
//! against that flat list: the tab offset of `active_window_index` plus that
//! window's `active_tab_index` (falling back to the document-level
//! `active_tab_index`), clamped into range.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
        }
    }

    /// A leaf pane in `cwd` with no startup commands and no focus. A builder
    /// convenience for constructing layouts to export.
    pub fn pane(cwd: Option<PathBuf>) -> LaunchLayout {
        LaunchLayout::Pane {
            cwd,
            commands: Vec::new(),
            focused: false,
        }
    }

    /// A split of `panes`. `vertical` matches Warp's `split_direction:
    /// vertical` (panes side by side). A builder convenience; callers should
    /// pass at least two panes (a single-pane split collapses on re-parse).
    pub fn split(vertical: bool, panes: Vec<LaunchLayout>) -> LaunchLayout {
        LaunchLayout::Split { vertical, panes }
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

    /// Every pane in document order (depth-first), useful for the app to
    /// count sessions.
    pub fn pane_count(&self) -> usize {
        self.tabs.iter().map(|t| t.layout.pane_count()).sum()
    }

    /// Serialize back to Warp-compatible launch-configuration YAML — the exact
    /// schema [`LaunchConfig::parse_yaml`] reads. Rift has a single window,
    /// so all tabs are written under one `windows` entry, and `active_tab` is
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

/// `~/.config/rift/launch_configs/<slug>.yaml`, never overwriting: if the
/// name's slug is taken, `-2`, `-3`, … is appended. Returns the path written.
/// Errors if [`launch_dir`] is unavailable (no config directory).
pub fn save(cfg: &LaunchConfig) -> std::io::Result<PathBuf> {
    let dir = launch_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory for launch configs",
        )
    })?;
    save_to_dir(cfg, &dir)
}

/// [`save`] against an explicit directory (also the unit-test entry point).
/// Creates `dir` if needed, writes `<slug>.yaml` without ever overwriting an
/// existing file (`-2`, `-3`, … suffixes), and returns the path written.
pub fn save_to_dir(cfg: &LaunchConfig, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let slug = slugify(&cfg.name);
    let body = cfg.to_yaml();

    let mut path = dir.join(format!("{slug}.yaml"));
    let mut n = 2u32;
    while path.exists() {
        path = dir.join(format!("{slug}-{n}.yaml"));
        n += 1;
    }
    std::fs::write(&path, body)?;
    Ok(path)
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
        } => OutLayout {
            cwd: cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
            commands: commands
                .iter()
                .map(|exec| OutCommand { exec: exec.clone() })
                .collect(),
            is_focused: *focused,
            split_direction: None,
            panes: None,
        },
        LaunchLayout::Split { vertical, panes } => OutLayout {
            cwd: None,
            commands: Vec::new(),
            is_focused: false,
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
fn expand_tilde(s: &str) -> PathBuf {
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

/// Every `*.yaml` / `*.yml` in [`launch_dir`], sorted by name. Files that
/// fail to parse are logged and skipped.
pub fn load_all() -> Vec<LaunchConfig> {
    match launch_dir() {
        Some(dir) => load_from_dir(&dir),
        None => Vec::new(),
    }
}

/// [`load_all`] against an explicit directory (also the unit-test entry
/// point, so tests never have to move `$XDG_CONFIG_HOME`).
pub fn load_from_dir(dir: &Path) -> Vec<LaunchConfig> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<LaunchConfig> = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|s| s.to_str());
        if !matches!(ext, Some("yaml") | Some("yml")) {
            continue;
        }
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("launch")
            .to_string();
        match std::fs::read_to_string(&p) {
            Ok(body) => match LaunchConfig::parse_yaml(&stem, &body) {
                Ok(c) => out.push(c),
                Err(err) => log::warn!("launch config {p:?}: {err}"),
            },
            Err(err) => log::warn!("launch config {p:?}: {err}"),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
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
    // `color` and any other Warp-only keys are ignored.
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
                    },
                    LaunchLayout::Pane {
                        cwd: Some(home()),
                        commands: vec![],
                        focused: false,
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
            }
        );
        assert_eq!(
            c.tabs[1].layout,
            LaunchLayout::Pane {
                cwd: Some(home()),
                commands: vec![],
                focused: false,
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
                    layout: LaunchLayout::Pane {
                        cwd: Some(PathBuf::from("/work/app")),
                        commands: vec!["nvim".into()],
                        focused: true,
                    },
                },
                LaunchTab {
                    title: None,
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
    fn save_to_dir_writes_and_suffixes_without_overwriting() {
        let dir = scratch("save");
        let cfg = LaunchConfig::parse_yaml("stem", FULL).unwrap();

        let p1 = save_to_dir(&cfg, &dir).unwrap();
        assert_eq!(p1.file_name().unwrap(), "dev-server.yaml");

        // Same name again → suffixed, original untouched.
        let p2 = save_to_dir(&cfg, &dir).unwrap();
        assert_eq!(p2.file_name().unwrap(), "dev-server-2.yaml");
        let p3 = save_to_dir(&cfg, &dir).unwrap();
        assert_eq!(p3.file_name().unwrap(), "dev-server-3.yaml");

        // The written file parses back to an equal config.
        let body = std::fs::read_to_string(&p1).unwrap();
        assert_eq!(LaunchConfig::parse_yaml("x", &body).unwrap(), cfg);

        std::fs::remove_dir_all(&dir).unwrap();
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
