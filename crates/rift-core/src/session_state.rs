/// Session persistence: reopen tabs with their working directories, custom
/// titles and colors, and full split layouts.
///
/// Written atomically on every structural change (there is no single reliable
/// exit hook across all four exit paths), read once at launch. The on-disk
/// format is TOML and is **backward compatible**: files written by older
/// builds (a bare `cwd` per tab, no `version`, no `layout`) still load, because
/// every field added since is optional with a serde default.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Schema version stamped into freshly built sessions. `1` = the legacy
/// `cwd`-only shape; `2` adds per-tab `title`, `color` and a nested `layout`;
/// `3` adds optional per-pane `transcript` ids (saved block output).
/// A file with no `version` key loads as [`LEGACY_VERSION`].
pub const CURRENT_VERSION: u32 = 3;

/// The version assumed for a file that predates the `version` key.
pub const LEGACY_VERSION: u32 = 1;

fn legacy_version() -> u32 {
    LEGACY_VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    /// Schema version; see [`CURRENT_VERSION`]. Legacy files without the key
    /// load as [`LEGACY_VERSION`]. [`SavedSession::default`] stamps
    /// [`CURRENT_VERSION`], so any session the app builds and saves is current.
    #[serde(default = "legacy_version")]
    pub version: u32,
    /// Index of the active tab into `tabs` (clamped by the caller on restore).
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    /// Window frame at save time (logical px); restored on launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SavedWindow>,
}

/// Window geometry (logical px) and state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
}

impl Default for SavedSession {
    fn default() -> Self {
        SavedSession {
            version: CURRENT_VERSION,
            active: 0,
            tabs: Vec::new(),
            window: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SavedTab {
    /// Working directory of the tab's focused pane. Retained as the top-level
    /// tab cwd even when `layout` is present, so an older build (which reads
    /// only this field) still reopens the tab in a sensible directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// User-set custom tab title. `None` means the title is derived from the
    /// running program, as usual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Tab color name (a theme color key), if the user assigned one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The tab's full pane tree. `None` is the legacy shape: a single unsplit
    /// pane whose directory is [`SavedTab::cwd`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<SavedLayout>,
    /// Saved block transcript of an unsplit tab's pane (split tabs keep one
    /// per leaf in `layout`). See `transcript.rs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    /// Session-service shell of an unsplit tab's pane (durable sessions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<u64>,
    /// When the tab's first command ran, in Unix seconds (the Elapsed chip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<u64>,
}

/// A tab's pane tree: either one leaf pane or a binary split of two subtrees.
/// Mirrors the app's runtime pane tree so a restored session rebuilds the exact
/// split layout.
///
/// Serialized with an internal `kind` tag, which the workspace `toml` crate
/// round-trips cleanly (see the tests):
///
/// ```toml
/// [tabs.layout]
/// kind = "Split"
/// axis = "Horizontal"
/// ratio = 0.5
///
/// [tabs.layout.first]
/// kind = "Pane"
/// cwd = "/a"
/// focused = true
///
/// [tabs.layout.second]
/// kind = "Pane"
/// focused = false
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SavedLayout {
    /// A leaf: one terminal pane.
    Pane {
        /// Working directory to reopen the pane in.
        #[serde(default)]
        cwd: Option<String>,
        /// Whether this pane held focus within its tab.
        #[serde(default)]
        focused: bool,
        /// Saved block transcript id, when transcripts are enabled.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript: Option<String>,
        /// Session-service shell to reattach (durable sessions).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        service: Option<u64>,
    },
    /// A split of exactly two children, divided along `axis` at `ratio`.
    Split {
        /// Orientation of the divider between `first` and `second`.
        axis: SavedAxis,
        /// Fraction of the split's extent given to `first`, in `0.0..=1.0`.
        ratio: f32,
        /// The first (top or left) subtree.
        first: Box<SavedLayout>,
        /// The second (bottom or right) subtree.
        second: Box<SavedLayout>,
    },
}

impl SavedLayout {
    /// A leaf pane. `focused` defaults to `false`; set it on the returned
    /// value if this pane held focus.
    pub fn pane(cwd: Option<String>) -> SavedLayout {
        SavedLayout::Pane {
            cwd,
            focused: false,
            transcript: None,
            service: None,
        }
    }

    /// Transcript id of the first leaf (the pane spawned for this subtree).
    pub fn first_transcript(&self) -> Option<&str> {
        match self {
            SavedLayout::Pane { transcript, .. } => transcript.as_deref(),
            SavedLayout::Split { first, .. } => first.first_transcript(),
        }
    }

    /// Session-service shell of the first leaf.
    pub fn first_service(&self) -> Option<u64> {
        match self {
            SavedLayout::Pane { service, .. } => *service,
            SavedLayout::Split { first, .. } => first.first_service(),
        }
    }

    /// A binary split of two subtrees along `axis`, `first` taking `ratio` of
    /// the extent.
    pub fn split(
        axis: SavedAxis,
        ratio: f32,
        first: SavedLayout,
        second: SavedLayout,
    ) -> SavedLayout {
        SavedLayout::Split {
            axis,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// The `cwd` of the focused leaf, if any leaf in the tree is focused,
    /// falling back to the first leaf's `cwd`. Handy for keeping
    /// [`SavedTab::cwd`] in sync with a `layout`.
    pub fn focused_cwd(&self) -> Option<&str> {
        match self {
            SavedLayout::Pane { cwd, focused, .. } => {
                if *focused {
                    cwd.as_deref()
                } else {
                    None
                }
            }
            SavedLayout::Split { first, second, .. } => {
                first.focused_cwd().or_else(|| second.focused_cwd())
            }
        }
    }

    /// Number of leaf panes in the tree.
    pub fn pane_count(&self) -> usize {
        match self {
            SavedLayout::Pane { .. } => 1,
            SavedLayout::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }
}

/// Split orientation. `Horizontal` = the two panes sit **side by side** (a
/// vertical divider between them); `Vertical` = the two panes are **stacked**
/// (a horizontal divider).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedAxis {
    /// Side by side, split left/right.
    Horizontal,
    /// Stacked, split top/bottom.
    Vertical,
}

pub fn session_path() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("session.toml"))
}

pub fn load() -> Option<SavedSession> {
    load_from(&session_path()?)
}

pub fn load_from(path: &std::path::Path) -> Option<SavedSession> {
    let body = std::fs::read_to_string(path).ok()?;
    toml::from_str(&body).ok()
}

/// Atomic write (tmp + rename) — this file is written frequently.
pub fn save(session: &SavedSession) {
    if let Some(path) = session_path() {
        let _ = save_to(&path, session);
    }
}

/// Atomic write of `session` to `path`: complete before it is visible.
pub fn save_to(path: &std::path::Path, session: &SavedSession) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = toml::to_string(session).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// Where windows leave their layouts for a restart (an update): one file
/// per window, `main-<pid>.toml` for a window that owns `session.toml`.
pub fn restart_dir() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("restart"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_toml() {
        let s = SavedSession {
            version: CURRENT_VERSION,
            tabs: vec![
                SavedTab {
                    cwd: Some("/tmp".into()),
                    ..Default::default()
                },
                SavedTab::default(),
            ],
            active: 1,
            window: Some(SavedWindow {
                x: 10.0,
                y: 20.0,
                width: 900.0,
                height: 600.0,
                maximized: false,
                fullscreen: true,
            }),
        };
        let body = toml::to_string(&s).unwrap();
        let back: SavedSession = toml::from_str(&body).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn roundtrip_nested_layout() {
        let layout = SavedLayout::split(
            SavedAxis::Horizontal,
            0.5,
            SavedLayout::Pane {
                cwd: Some("/a".into()),
                focused: true,
                transcript: Some("1-2-3".into()),
                service: Some(9),
            },
            SavedLayout::split(
                SavedAxis::Vertical,
                0.25,
                SavedLayout::pane(Some("/b".into())),
                SavedLayout::pane(None),
            ),
        );
        assert_eq!(layout.pane_count(), 3);
        assert_eq!(layout.focused_cwd(), Some("/a"));
        assert_eq!(layout.first_transcript(), Some("1-2-3"));
        assert_eq!(layout.first_service(), Some(9));

        let s = SavedSession {
            version: CURRENT_VERSION,
            active: 0,
            tabs: vec![SavedTab {
                cwd: Some("/a".into()),
                title: Some("build".into()),
                color: Some("blue".into()),
                layout: Some(layout),
                transcript: None,
                service: None,
                started: Some(1_791_938_924),
            }],
            window: None,
        };

        let body = toml::to_string(&s).unwrap();
        // Prove the enum actually round-trips through the workspace toml crate.
        let back: SavedSession = toml::from_str(&body).unwrap();
        assert_eq!(back, s, "toml body was:\n{body}");
    }

    #[test]
    fn old_format_parses_with_defaults() {
        // A file written by the pre-layout build: no `version`, tabs carry only
        // `cwd`.
        let body = "active = 1\n\n[[tabs]]\ncwd = \"/tmp\"\n\n[[tabs]]\n";
        let s: SavedSession = toml::from_str(body).unwrap();
        assert_eq!(s.version, LEGACY_VERSION);
        assert_eq!(s.active, 1);
        assert_eq!(s.tabs.len(), 2);
        assert_eq!(s.tabs[0].cwd.as_deref(), Some("/tmp"));
        assert_eq!(s.tabs[0].title, None);
        assert_eq!(s.tabs[0].color, None);
        assert_eq!(s.tabs[0].layout, None);
        assert_eq!(s.tabs[1].cwd, None);
    }

    #[test]
    fn empty_document_is_default_ish() {
        // An empty file loads: no tabs, legacy version, active 0.
        let s: SavedSession = toml::from_str("").unwrap();
        assert_eq!(s.version, LEGACY_VERSION);
        assert_eq!(s.active, 0);
        assert!(s.tabs.is_empty());
    }

    #[test]
    fn default_session_is_current_version() {
        assert_eq!(SavedSession::default().version, CURRENT_VERSION);
    }
}
