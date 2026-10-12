//! External editors that open files from Rift: ⌘-click file links
//! (`link_editor`) and the file explorer / code review (`file_editor`).
//! A setting holds an editor id; `SYSTEM` (the OS opener) and `RIFT` (the
//! built-in viewer) are handled by the app, the rest launch an installed
//! application with the file as an argument (never through a shell).

use std::path::{Path, PathBuf};

/// The OS opener (`open` / `xdg-open`): the file's default application.
pub const SYSTEM: &str = "system";
/// Rift's read-only viewer pane.
pub const RIFT: &str = "rift";

/// An external editor Rift can launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Editor {
    /// Config value.
    pub id: &'static str,
    pub label: &'static str,
    /// macOS application bundle name (`open -a <app> <file>`).
    pub mac_app: &'static str,
    /// Command on `PATH` (Linux, and macOS when the app bundle is absent).
    pub cli: &'static str,
}

pub const EDITORS: &[Editor] = &[
    Editor {
        id: "vscode",
        label: "VS Code",
        mac_app: "Visual Studio Code",
        cli: "code",
    },
    Editor {
        id: "cursor",
        label: "Cursor",
        mac_app: "Cursor",
        cli: "cursor",
    },
    Editor {
        id: "windsurf",
        label: "Windsurf",
        mac_app: "Windsurf",
        cli: "windsurf",
    },
    Editor {
        id: "zed",
        label: "Zed",
        mac_app: "Zed",
        cli: "zed",
    },
    Editor {
        id: "sublime",
        label: "Sublime Text",
        mac_app: "Sublime Text",
        cli: "subl",
    },
    Editor {
        id: "intellij",
        label: "IntelliJ IDEA",
        mac_app: "IntelliJ IDEA",
        cli: "idea",
    },
    Editor {
        id: "phpstorm",
        label: "PhpStorm",
        mac_app: "PhpStorm",
        cli: "phpstorm",
    },
    Editor {
        id: "webstorm",
        label: "WebStorm",
        mac_app: "WebStorm",
        cli: "webstorm",
    },
    Editor {
        id: "rustrover",
        label: "RustRover",
        mac_app: "RustRover",
        cli: "rustrover",
    },
];

pub fn by_id(id: &str) -> Option<&'static Editor> {
    EDITORS
        .iter()
        .find(|e| e.id.eq_ignore_ascii_case(id.trim()))
}

/// Shown name of a setting value: an editor's label, the built-in choices,
/// or the raw value for an unknown id.
pub fn label(id: &str) -> String {
    match id.trim() {
        "" | SYSTEM => "System default".into(),
        RIFT => "Rift viewer".into(),
        other => by_id(other).map_or_else(|| other.to_string(), |e| e.label.to_string()),
    }
}

/// The macOS bundle of `app` in /Applications or ~/Applications.
fn mac_bundle(app: &str) -> Option<PathBuf> {
    let name = format!("{app}.app");
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
        .into_iter()
        .map(|r| r.join(&name))
        .find(|p| p.is_dir())
}

/// `cli` on `PATH`.
fn on_path(cli: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(cli))
        .find(|p| p.is_file())
}

impl Editor {
    pub fn installed(&self) -> bool {
        (cfg!(target_os = "macos") && mac_bundle(self.mac_app).is_some())
            || on_path(self.cli).is_some()
    }

    /// Program and argv that open `path` in this editor.
    pub fn command(&self, path: &Path) -> (String, Vec<std::ffi::OsString>) {
        if cfg!(target_os = "macos") && mac_bundle(self.mac_app).is_some() {
            return (
                "open".into(),
                vec!["-a".into(), self.mac_app.into(), path.into()],
            );
        }
        (self.cli.into(), vec![path.into()])
    }
}

/// The editors installed on this machine, in `EDITORS` order.
pub fn installed() -> Vec<&'static Editor> {
    EDITORS.iter().filter(|e| e.installed()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_cover_builtins_editors_and_unknown_ids() {
        assert_eq!(label(""), "System default");
        assert_eq!(label(SYSTEM), "System default");
        assert_eq!(label(RIFT), "Rift viewer");
        assert_eq!(label("vscode"), "VS Code");
        assert_eq!(label(" Zed "), "Zed");
        assert_eq!(label("nano"), "nano");
    }

    #[test]
    fn ids_are_unique_and_distinct_from_builtins() {
        for (i, e) in EDITORS.iter().enumerate() {
            assert!(e.id != SYSTEM && e.id != RIFT);
            assert!(EDITORS[i + 1..].iter().all(|o| o.id != e.id));
            assert_eq!(by_id(e.id), Some(e));
        }
        assert!(by_id("nano").is_none());
    }

    #[test]
    fn command_passes_the_path_as_one_argument() {
        let e = by_id("vscode").unwrap();
        let path = Path::new("/tmp/a b; rm -rf ~.txt");
        let (_, args) = e.command(path);
        assert_eq!(args.last().map(|a| a.as_os_str()), Some(path.as_os_str()));
    }
}
