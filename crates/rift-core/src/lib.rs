pub mod actions;
pub mod block;
pub mod changelog;
pub mod cli_install;
pub mod code_highlight;
pub mod command_history;
pub mod completion;
mod completion_cache;
pub mod completion_specs;
pub mod config;
pub mod config_lint;
pub mod corrections;
pub mod docker;
pub mod editing;
pub mod editors;
pub mod fs_tree;
pub mod generated;
pub mod git;
pub mod git_write;
pub mod github;
pub mod highlight;
pub mod history;
pub mod ignored_suggestions;
pub mod inspect;
pub mod keybindings;
pub mod launch;
pub mod markdown;
pub mod notifications;
pub mod osc;
mod process;
pub mod profiles;
pub mod prompts;
pub mod remote_complete;
pub mod result_search;
pub mod review;
pub mod runtimes;
pub mod secrets;
pub mod session_state;
pub mod shell_complete;
pub mod shell_env;
pub mod shell_integration;
pub mod shell_widget;
pub mod ssh;
pub mod tab_clock;
pub mod tab_color;
pub mod theme;
pub mod transcript;
pub mod update;
pub mod uri;
pub mod watch;

/// Tests that change the process environment (`XDG_DATA_HOME`) and those
/// that read it hold this, one at a time.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub use block::{AbsLine, Block, BlockId, BlockState, BlockStore, Restored};
pub use config::{
    AmbiguousWidth, AppIcon, Bell, BlockSpacing, BlocksStart, CloseOnExit, Config, CursorStyle,
    ElapsedCounts, HotkeyEdge, InputPosition, LaunchMode, NewTabPlacement, Padding, PromptChip,
    PromptMode, RichInputSubmit, ScrollbarMode, ShellWidgets, TabBarVisibility, TabBehavior,
    TabCloseButton, TabCloseSide, TabLayout, TabStyle, TabSwitchOrder, ToolbarItem,
};
pub use keybindings::{Binding, Keymap};
pub use osc::{Marker, OscScanner, Progress, ProgressState, host_is_local};
pub use theme::{Color, Theme};
