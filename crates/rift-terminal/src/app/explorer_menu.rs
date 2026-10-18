//! The file explorer's right-click menu: open, copy the path or the path
//! relative to the explorer's root, a new file in the folder (its name
//! asked in the palette's text input: a name only, never a path out of the
//! folder), and reveal in Finder / the file manager.

use super::*;
use crate::views::context_menu::MenuItem;

impl App {
    /// A right-click on explorer row `i`: its menu at `pos`.
    pub(super) fn open_explorer_menu(&mut self, i: usize, pos: (f32, f32)) {
        let Some(row) = self.win.explorer.rows.get(i).cloned() else {
            return;
        };
        if row.more {
            return;
        }
        self.win.explorer.selected = Some(row.path.clone());
        let items = vec![
            MenuItem::new(
                if row.is_dir {
                    "Expand or collapse"
                } else {
                    "Open"
                },
                MenuAction::ExplorerOpen,
            ),
            MenuItem::new("Copy path", MenuAction::ExplorerCopyPath).sep(),
            MenuItem::new("Copy relative path", MenuAction::ExplorerCopyRelativePath),
            MenuItem::new("New file…", MenuAction::ExplorerNewFile).sep(),
            MenuItem::new(
                if cfg!(target_os = "macos") {
                    "Reveal in Finder"
                } else {
                    "Show in file manager"
                },
                MenuAction::ExplorerReveal,
            ),
        ];
        self.win.explorer_menu = Some(row);
        self.win.context_menu = ContextMenuState {
            open: true,
            pane: 0,
            block: 0,
            pos,
            items,
            selected: None,
        };
    }

    /// Run an explorer menu action; false for any other action.
    pub(super) fn run_explorer_menu(&mut self, action: MenuAction) -> bool {
        if !matches!(
            action,
            MenuAction::ExplorerOpen
                | MenuAction::ExplorerCopyPath
                | MenuAction::ExplorerCopyRelativePath
                | MenuAction::ExplorerNewFile
                | MenuAction::ExplorerReveal
        ) {
            return false;
        }
        let Some(row) = self.win.explorer_menu.take() else {
            return true;
        };
        match action {
            MenuAction::ExplorerOpen => {
                if row.is_dir {
                    self.win.explorer.toggle(&row.path);
                } else {
                    let choice = self.config.file_editor.clone();
                    self.open_file_in(&row.path, &choice);
                }
            }
            MenuAction::ExplorerCopyPath => {
                if self.copy_to_clipboard(row.path.to_string_lossy().into_owned()) {
                    self.toast_done("Path copied".into());
                }
            }
            MenuAction::ExplorerCopyRelativePath => {
                let rel = relative_to(&row.path, self.win.explorer.root.as_deref());
                if self.copy_to_clipboard(rel) {
                    self.toast_done("Relative path copied".into());
                }
            }
            MenuAction::ExplorerNewFile => {
                let dir = if row.is_dir {
                    row.path.clone()
                } else {
                    row.path
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| row.path.clone())
                };
                self.open_new_file_prompt(dir);
            }
            MenuAction::ExplorerReveal => {
                if self.headless {
                    // a file manager window would take the user's focus
                    self.toast(format!("Headless: not revealing {}", row.path.display()));
                } else if let Err(e) = reveal_external(&row.path) {
                    self.toast_error(format!("Could not reveal {}: {e}", row.path.display()));
                }
            }
            _ => {}
        }
        true
    }

    /// Ask for the new file's name in `dir` (the palette's text input; the
    /// row under it shows the path it creates).
    fn open_new_file_prompt(&mut self, dir: std::path::PathBuf) {
        self.win.save_target = Some((NEW_FILE.into(), String::new()));
        self.open_palette(crate::views::palette::PaletteMode::SavePath);
        self.win.new_file_dir = Some(dir);
        self.win.palette.query.clear();
        self.refresh_palette();
    }

    /// Enter in the new-file prompt: create the named file in the folder
    /// the menu was on; an existing file (or a link) is left alone.
    pub(super) fn create_typed_file(&mut self, dir: std::path::PathBuf) {
        let path = match new_file_path(&self.win.palette.query, &dir) {
            Ok(path) => path,
            Err(why) => {
                self.toast_error(format!("New file: {why}"));
                return;
            }
        };
        // the folder must still be inside the explorer's root, as its real
        // path: a symlinked folder leading out of it is refused
        if let Some(root) = self.win.explorer.root.as_deref() {
            let real = |p: &std::path::Path| std::fs::canonicalize(p).ok();
            let inside =
                matches!((real(&dir), real(root)), (Some(d), Some(r)) if d.starts_with(&r));
            if !inside {
                self.toast_error(format!(
                    "New file: {} leads out of the explorer's folder",
                    crate::views::editor::shorten_home(&dir.to_string_lossy())
                ));
                return;
            }
        }
        let shown = crate::views::editor::shorten_home(&path.to_string_lossy());
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        match created {
            Ok(_) => {
                self.toast_done(format!("Created {shown}"));
                // the folder opens to show it
                if let Some(parent) = path.parent() {
                    self.win.explorer.expanded.insert(parent.to_path_buf());
                }
                self.win.explorer.refresh();
                self.win.explorer.selected = Some(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                self.toast_error(format!("{shown} already exists"));
            }
            Err(e) => self.toast_error(format!("Could not create {shown}: {e}")),
        }
    }
}

/// `save_target`'s label while the save prompt asks for a new file's name.
pub(super) const NEW_FILE: &str = "a new file";

/// `path` relative to the explorer's `root` (`.` for the root itself); a
/// path outside it stays absolute.
fn relative_to(path: &std::path::Path, root: Option<&std::path::Path>) -> String {
    match root.and_then(|r| path.strip_prefix(r).ok()) {
        Some(rel) if rel.as_os_str().is_empty() => ".".into(),
        Some(rel) => rel.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

/// The new file's path for the typed name: in `dir`, and only there. A
/// path (any `/`, so an absolute one too), `.`, `..` or nothing typed is
/// refused, with why.
pub(super) fn new_file_path(
    typed: &str,
    dir: &std::path::Path,
) -> Result<std::path::PathBuf, &'static str> {
    let name = typed.trim();
    if name.is_empty() {
        return Err("type the new file's name");
    }
    if name.contains('/') || name.contains('\0') {
        return Err("a name only, without /");
    }
    if name == "." || name == ".." {
        return Err("not a file name");
    }
    Ok(dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn relative_paths_are_from_the_explorer_root() {
        let root = Path::new("/w/repo");
        assert_eq!(
            relative_to(Path::new("/w/repo/src/a.rs"), Some(root)),
            "src/a.rs"
        );
        assert_eq!(relative_to(root, Some(root)), ".");
        assert_eq!(
            relative_to(Path::new("/etc/hosts"), Some(root)),
            "/etc/hosts"
        );
        assert_eq!(relative_to(Path::new("/w/repo/x"), None), "/w/repo/x");
    }

    #[test]
    fn new_file_names_land_in_the_folder_only() {
        let dir = Path::new("/w/repo/src");
        assert_eq!(
            new_file_path(" mod.rs ", dir).as_deref(),
            Ok(Path::new("/w/repo/src/mod.rs"))
        );
        // `~` is a name like any other here, not home
        assert_eq!(
            new_file_path("~", dir).as_deref(),
            Ok(Path::new("/w/repo/src/~"))
        );
        for bad in [
            "",
            "  ",
            ".",
            "..",
            "../x",
            "a/b",
            "/tmp/x.txt",
            "~/x",
            "sub/",
            "x\0y",
        ] {
            assert!(new_file_path(bad, dir).is_err(), "{bad:?}");
        }
    }
}
