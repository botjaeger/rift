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
/// `3` adds optional per-pane `transcript` ids (saved block output); `4`
/// adds `other_windows` (every window after the first); `5` adds per-pane
/// `launch` (profile name, own shell and arguments); `6` adds pinned tabs,
/// tab groups (`groups`, per-tab `group`) and read-only panes; `7` adds
/// window names (`window_name`, per other window `name`); `8` added `kept`
/// (Rift 0.17–0.23 kept the shells running after a quit; now ignored).
/// A file with no `version` key loads as [`LEGACY_VERSION`].
pub const CURRENT_VERSION: u32 = 8;

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
    /// Every window after the first (version 4). The first window stays in
    /// `active`, `tabs` and `window` above, so a build that predates this
    /// key still reopens it (and ignores the rest).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_windows: Vec<SavedWindowTabs>,
    /// The first window's tab groups (version 6), named by index in
    /// [`SavedTab::group`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<SavedGroup>,
    /// The first window's name (version 7; "Rename window…").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_name: Option<String>,
}

/// A named, coloured tab group; its tabs name it by index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SavedGroup {
    #[serde(default)]
    pub name: String,
    /// Tab color name (as [`SavedTab::color`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Its tabs are folded away behind its header.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

/// One window of a session: its tabs, the active one and its frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SavedWindowTabs {
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SavedWindow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<SavedGroup>,
    /// Its name ("Rename window…"), shown as its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl SavedSession {
    /// Every window, the first one first (a file before version 4 has one).
    pub fn windows(&self) -> Vec<SavedWindowTabs> {
        let first = SavedWindowTabs {
            active: self.active,
            tabs: self.tabs.clone(),
            window: self.window,
            groups: self.groups.clone(),
            name: self.window_name.clone(),
        };
        std::iter::once(first)
            .chain(self.other_windows.iter().cloned())
            .collect()
    }

    /// The session of `windows` (the first one first, at the top level).
    pub fn of_windows(mut windows: Vec<SavedWindowTabs>) -> SavedSession {
        if windows.is_empty() {
            return SavedSession::default();
        }
        let first = windows.remove(0);
        SavedSession {
            active: first.active,
            tabs: first.tabs,
            window: first.window,
            groups: first.groups,
            window_name: first.name,
            other_windows: windows,
            ..Default::default()
        }
    }
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
            other_windows: Vec::new(),
            groups: Vec::new(),
            window_name: None,
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
    /// Seconds a command ran in the tab (the Elapsed chip with
    /// `elapsed_counts = "running"`); absent in older files, which load as 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counted: Option<u64>,
    /// What an unsplit tab's pane was opened with (split tabs keep one per
    /// leaf in `layout`); absent in older files: the default launch.
    #[serde(
        default,
        skip_serializing_if = "crate::profiles::PaneLaunch::is_default"
    )]
    pub launch: crate::profiles::PaneLaunch,
    /// Pinned: kept first in the strip, compact (version 6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// Index of its group in the window's `groups` (version 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<usize>,
    /// An unsplit tab's pane refuses input (split tabs keep one per leaf).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
}

/// A tab's pane tree: either one leaf pane or a binary split of two subtrees.
/// The app's runtime splits hold any number of weighted panes; a row of n is
/// saved as a chain of two-pane splits nested in `second`, and same-axis
/// chains flatten back into one row with each pane's share on restore.
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
        /// What the pane was opened with (profile name, own shell, args).
        #[serde(
            default,
            skip_serializing_if = "crate::profiles::PaneLaunch::is_default"
        )]
        launch: crate::profiles::PaneLaunch,
        /// The pane refuses input (read-only).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        read_only: bool,
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
            launch: Default::default(),
            read_only: false,
        }
    }

    /// What the first leaf (the pane spawned for this subtree) was opened
    /// with.
    pub fn first_launch(&self) -> crate::profiles::PaneLaunch {
        match self {
            SavedLayout::Pane { launch, .. } => launch.clone(),
            SavedLayout::Split { first, .. } => first.first_launch(),
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

// --- crash recovery: was the last run ended on purpose? ------------------------------------

/// `<data dir>/running`: written at launch with the window process's pid and
/// removed by every ordinary way out (a quit, an update's restart). A marker
/// whose process is gone means the last run ended in a crash, a kill or a
/// power loss.
fn running_marker() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("running"))
}

/// Record that this process is running.
pub fn mark_running() {
    let Some(path) = running_marker() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, std::process::id().to_string());
}

/// This process is ending on purpose: forget that it ran. Another process's
/// marker is left alone.
pub fn mark_clean_exit() {
    let Some(path) = running_marker() else {
        return;
    };
    if std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        == Some(std::process::id())
    {
        let _ = std::fs::remove_file(path);
    }
}

/// The last run left its marker and its process is gone: it did not end on
/// purpose. False when there is no marker, or the process in it still runs
/// (another Rift, or this one).
pub fn previous_run_crashed() -> bool {
    let Some(pid) = running_marker()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
    else {
        return false;
    };
    pid != std::process::id() && !crate::sys::process_alive(pid)
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
    crate::sys::rename(&tmp, path)
}

/// Where windows leave their layouts for a restart (an update): one file
/// per window, `main-<pid>.toml` for a window that owns `session.toml`.
pub fn restart_dir() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("restart"))
}

#[cfg(test)]
mod tests {
    /// `data_dir()` follows XDG_DATA_HOME: a scratch one per test, one test at a time.
    fn with_data_dir(f: impl FnOnce()) {
        let _one = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("rift-running-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: serialised by TEST_ENV_LOCK, which the tests reading the
        // variable hold too
        unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };
        f();
        unsafe { std::env::remove_var("XDG_DATA_HOME") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_that_ended_on_purpose_leaves_no_marker() {
        with_data_dir(|| {
            assert!(!previous_run_crashed(), "no marker: nothing crashed");
            mark_running();
            // this very process runs: not a crash
            assert!(!previous_run_crashed());
            mark_clean_exit();
            assert!(!running_marker().unwrap().exists());
            assert!(!previous_run_crashed());
        });
    }

    #[test]
    #[cfg(unix)]
    fn a_marker_of_a_gone_process_means_the_last_run_crashed() {
        with_data_dir(|| {
            let mut child = std::process::Command::new("/bin/sleep")
                .arg("30")
                .spawn()
                .unwrap();
            let pid = child.id();
            let marker = running_marker().unwrap();
            std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            std::fs::write(&marker, pid.to_string()).unwrap();
            // the process lives: another Rift is running, not a crash
            assert!(!previous_run_crashed());
            child.kill().unwrap();
            child.wait().unwrap();
            assert!(previous_run_crashed());
            // a clean exit of ANOTHER process leaves its marker alone
            mark_clean_exit();
            assert!(running_marker().unwrap().exists());
            // the next launch marks itself running: no longer a crash
            mark_running();
            assert!(!previous_run_crashed());
        });
    }

    use super::*;

    #[test]
    fn roundtrip_toml() {
        let s = SavedSession {
            version: CURRENT_VERSION,
            tabs: vec![
                SavedTab {
                    cwd: Some("/tmp".into()),
                    launch: crate::profiles::PaneLaunch::profile("fish"),
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
            other_windows: Vec::new(),
            window_name: None,
            groups: Vec::new(),
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
                launch: crate::profiles::PaneLaunch {
                    profile: Some("dev box".into()),
                    shell: Some("/bin/b a\"sh".into()),
                    args: vec!["-c".into(), "it's \"quoted\"".into()],
                    theme: Some("tokyo-night".into()),
                },
                read_only: true,
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
        assert_eq!(layout.first_launch().profile.as_deref(), Some("dev box"));

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
                counted: Some(7200),
                launch: Default::default(),
                pinned: true,
                group: Some(0),
                read_only: false,
            }],
            window: None,
            other_windows: Vec::new(),
            window_name: Some("main".into()),
            groups: vec![SavedGroup {
                name: "Servers".into(),
                color: Some("green".into()),
                collapsed: true,
            }],
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
        // no counted time in an old file: the clock starts at 0
        assert_eq!(s.tabs[0].counted, None);
        let t = crate::tab_clock::TabClock::from_secs(s.tabs[0].counted.unwrap_or(0));
        assert_eq!(
            t.total(std::time::Instant::now()),
            std::time::Duration::ZERO
        );
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

    fn frame(x: f64) -> Option<SavedWindow> {
        Some(SavedWindow {
            x,
            y: 40.0,
            width: 800.0,
            height: 500.0,
            maximized: false,
            fullscreen: false,
        })
    }

    #[test]
    fn every_window_round_trips() {
        let tab = |cwd: &str| SavedTab {
            cwd: Some(cwd.into()),
            ..Default::default()
        };
        let split = SavedTab {
            layout: Some(SavedLayout::split(
                SavedAxis::Vertical,
                0.5,
                SavedLayout::pane(Some("/c".into())),
                SavedLayout::Pane {
                    cwd: Some("/d".into()),
                    focused: true,
                    transcript: None,
                    service: Some(7),
                    launch: Default::default(),
                    read_only: false,
                },
            )),
            ..tab("/d")
        };
        let windows = vec![
            SavedWindowTabs {
                active: 1,
                tabs: vec![tab("/a"), tab("/b")],
                window: frame(10.0),
                groups: Vec::new(),
                name: Some("first".into()),
            },
            SavedWindowTabs {
                active: 0,
                tabs: vec![split],
                window: frame(200.0),
                groups: Vec::new(),
                name: Some("logs".into()),
            },
            SavedWindowTabs {
                active: 0,
                tabs: vec![tab("/e")],
                window: None,
                groups: Vec::new(),
                name: None,
            },
        ];
        let s = SavedSession::of_windows(windows.clone());
        assert_eq!(s.version, CURRENT_VERSION);
        // the first window stays where a version-3 build reads it
        assert_eq!((s.active, s.tabs.len(), s.window), (1, 2, frame(10.0)));
        assert_eq!(s.window_name.as_deref(), Some("first"));
        let body = toml::to_string(&s).unwrap();
        let back: SavedSession = toml::from_str(&body).unwrap();
        assert_eq!(back, s, "toml body was:\n{body}");
        assert_eq!(back.windows(), windows);
        // a build that predates `other_windows` reads the first window alone
        #[derive(Deserialize)]
        struct V3 {
            active: usize,
            tabs: Vec<SavedTab>,
        }
        let old: V3 = toml::from_str(&body).unwrap();
        assert_eq!((old.active, old.tabs.len()), (1, 2));
        assert_eq!(old.tabs[0].cwd.as_deref(), Some("/a"));
    }

    #[test]
    fn a_version_3_file_is_one_window() {
        let body = "version = 3\nactive = 1\n\n[[tabs]]\ncwd = \"/a\"\n\n[[tabs]]\ncwd = \"/b\"\n\n\
                    [window]\nx = 1.0\ny = 2.0\nwidth = 900.0\nheight = 600.0\n";
        let s: SavedSession = toml::from_str(body).unwrap();
        assert_eq!(s.version, 3);
        let windows = s.windows();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].active, 1);
        assert_eq!(windows[0].tabs.len(), 2);
        assert_eq!(windows[0].window.map(|w| w.width), Some(900.0));
        assert_eq!(SavedSession::of_windows(Vec::new()).tabs.len(), 0);
    }
}
