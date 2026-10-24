//! `rift://` links (`rift_core::uri`): from macOS (an Apple Event, see
//! `crate::links`), a second process (the control socket), the command line
//! or the script driver. They open tabs, windows, saved layouts, a file in
//! the editor and Settings. A link runs no command of its own; a saved
//! layout whose panes run commands asks first (any web page can open a
//! link). Folders opened from outside (`rift-terminal <dir>`, Finder, the
//! Dock) arrive as `new_tab` / `new_window` links too.

use super::*;
use rift_core::uri::UriAction;

/// At most this many links in [`LINK_WINDOW`]; more are dropped (a page
/// that opens links in a loop cannot flood the screen with tabs).
const LINK_BURST: usize = 10;
const LINK_WINDOW: Duration = Duration::from_secs(3);

impl App {
    /// A fresh start's first tab: the first link Rift was started with when
    /// it opens a tab or window (a folder named on the command line or
    /// dropped on the Dock icon, or a bare `--new-window`), which is then
    /// done; in that folder, else where a new tab starts.
    pub(super) fn spawn_first_tab(&mut self) {
        self.collect_start_links();
        let first = self
            .start_links
            .first()
            .and_then(|l| match rift_core::uri::parse(l) {
                Ok(UriAction::NewTab(dir) | UriAction::NewWindow(dir)) => Some(dir),
                _ => None,
            });
        let folder = match first {
            Some(dir) => {
                self.start_links.remove(0);
                dir
            }
            None => None,
        };
        self.spawn_profile_tab(folder, self.start_dir());
    }

    /// The links Rift was started with that the first tab did not take
    /// (after a restored session: each its own tab).
    pub(super) fn open_start_links(&mut self, event_loop: &ActiveEventLoop) {
        self.collect_start_links();
        for link in std::mem::take(&mut self.start_links) {
            self.open_link(&link, event_loop);
        }
    }

    /// Add the links macOS handed over before the first window was up.
    fn collect_start_links(&mut self) {
        #[cfg(target_os = "macos")]
        self.start_links.extend(crate::links::take_launch_links());
    }

    /// Open a `rift://` link in the last focused ordinary window (one is
    /// opened when only the drop-down window exists); a link Rift refuses
    /// says why in a toast.
    pub(super) fn open_link(&mut self, link: &str, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.link_times
            .retain(|t| now.duration_since(*t) < LINK_WINDOW);
        if self.link_times.len() >= LINK_BURST {
            log::warn!("link dropped: too many links at once");
            return;
        }
        self.link_times.push(now);
        // a pane's own link (`RIFT_PANE_URL`): wherever it is, the
        // drop-down window included; it opens nothing
        if let Ok(UriAction::FocusPane(target)) = rift_core::uri::parse(link) {
            self.focus_pane_link(&target);
            return;
        }
        self.enter_focused_window();
        // never into the drop-down window: another ordinary one, else a new one
        let mut fresh_window = false;
        if self.win.dropdown.is_some() {
            self.enter_where(|w| w.dropdown.is_none() && w.window.is_some());
            if self.win.dropdown.is_some() {
                self.open_window(event_loop, None);
                fresh_window = true;
            }
        }
        let action = match rift_core::uri::parse(link) {
            Ok(a) => a,
            Err(e) => {
                log::warn!("link refused: {e}");
                if fresh_window {
                    self.spawn_profile_tab(None, self.start_dir());
                }
                self.toast_error(format!("Link refused — {}", shown(&e)));
                self.redraw_and_raise();
                return;
            }
        };
        log::info!("link: {action:?}");
        match action {
            UriAction::NewTab(cwd) => self.spawn_profile_tab(cwd, self.start_dir()),
            // a window just opened for the link is that window
            UriAction::NewWindow(cwd) if fresh_window => {
                self.spawn_profile_tab(cwd, self.start_dir())
            }
            UriAction::NewWindow(cwd) => self.open_new_window_at(event_loop, cwd),
            UriAction::Launch { name, new_window } => {
                self.launch_configs = rift_core::launch::load_all();
                let found = self
                    .launch_configs
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(&name))
                    .cloned();
                let Some(cfg) = found else {
                    if fresh_window {
                        self.spawn_profile_tab(None, self.start_dir());
                    }
                    self.toast_error(format!("No launch configuration named {}", shown(&name)));
                    self.redraw_and_raise();
                    return;
                };
                let commands = self.launch_link_commands(&cfg);
                if commands.is_empty() {
                    self.launch_from_link(&cfg, new_window && !fresh_window, event_loop);
                } else {
                    if fresh_window {
                        self.spawn_profile_tab(None, self.start_dir());
                    }
                    self.win.confirm = Some(ConfirmClose {
                        opened: Instant::now(),
                        focus_cancel: true,
                        action: ConfirmAction::LaunchLink {
                            config: Box::new(cfg),
                            new_window: new_window && !fresh_window,
                        },
                        commands,
                    });
                }
            }
            UriAction::OpenFile { path, line, col } => {
                if fresh_window {
                    self.spawn_profile_tab(None, self.start_dir());
                }
                let choice = self.config.link_editor.clone();
                self.open_file_at(&path, &choice, line.map(|l| (l, col)));
            }
            UriAction::Settings(query) => {
                if fresh_window {
                    self.spawn_profile_tab(None, self.start_dir());
                }
                self.win.settings.open_page(self.win.settings.page);
                if let Some(q) = query {
                    self.win.settings.query =
                        q.chars().filter(|c| !c.is_control()).take(200).collect();
                }
            }
            // taken above, before any window is chosen
            UriAction::FocusPane(target) => self.focus_pane_link(&target),
        }
        self.save_session();
        self.redraw_and_raise();
    }

    /// Preview every saved startup action, including executable/argv and
    /// environment overrides resolved through the same profile rules as spawn.
    /// A plain default shell needs no extra confirmation, as with a tab link.
    fn launch_link_commands(&self, cfg: &rift_core::launch::LaunchConfig) -> Vec<String> {
        fn collect(app: &App, layout: &rift_core::launch::LaunchLayout, out: &mut Vec<String>) {
            match layout {
                rift_core::launch::LaunchLayout::Pane {
                    commands, launch, ..
                } => {
                    if let Ok(resolved) = app.resolve_launch(launch)
                        && (resolved.program.is_some()
                            || !resolved.args.is_empty()
                            || !resolved.env.is_empty())
                    {
                        let program = resolved.program.unwrap_or_else(|| {
                            rift_core::shell_integration::resolve_shell(app.config.shell.as_deref())
                        });
                        let mut argv = std::iter::once(program)
                            .chain(resolved.args)
                            .map(|s| input::shell_quote(&s))
                            .collect::<Vec<_>>()
                            .join(" ");
                        if !resolved.env.is_empty() {
                            // Do not put profile secrets into the confirmation.
                            argv.push_str(" (custom environment)");
                        }
                        out.push(argv);
                    }
                    out.extend(commands.iter().cloned());
                }
                rift_core::launch::LaunchLayout::Split { panes, .. } => {
                    panes.iter().for_each(|p| collect(app, p, out));
                }
            }
        }
        let mut commands = Vec::new();
        cfg.tabs
            .iter()
            .for_each(|t| collect(self, &t.layout, &mut commands));
        commands
    }

    /// `rift://action/focus`: raise the window holding the pane and focus
    /// its tab and pane; one that is gone (closed, or a local shell of an
    /// earlier run) says so in a toast in the last focused window.
    fn focus_pane_link(&mut self, target: &rift_core::uri::PaneRef) {
        use rift_core::uri::PaneRef;
        let found = match target {
            PaneRef::Local { run, pane } => (run == rift_core::uri::run_token())
                .then_some(*pane)
                .filter(|&id| self.all_panes().any(|p| p.session.id == id)),
            PaneRef::Service(session) => self
                .all_panes()
                .find(|p| p.session.service_id() == Some(*session))
                .map(|p| p.session.id),
        };
        let Some(pane) = found.filter(|&id| self.enter_pane_window(id)) else {
            self.enter_focused_window();
            self.toast_error(match target {
                PaneRef::Local { run, .. } if run != rift_core::uri::run_token() => {
                    "That pane belonged to an earlier Rift run".to_string()
                }
                _ => "That pane is no longer open".to_string(),
            });
            self.redraw_and_raise();
            return;
        };
        self.focus_pane_here(pane);
        self.redraw_and_raise();
    }

    /// Open a launch configuration a link named (here, or in a window of
    /// its own).
    pub(super) fn launch_from_link(
        &mut self,
        cfg: &rift_core::launch::LaunchConfig,
        new_window: bool,
        event_loop: &ActiveEventLoop,
    ) {
        if new_window {
            self.open_window(event_loop, None);
        }
        self.apply_launch_config(cfg);
        self.save_session();
        self.redraw_and_raise();
    }

    /// Repaint the served window and bring it to the front (a link is
    /// opened from another app); a scripted run is never raised.
    fn redraw_and_raise(&mut self) {
        let Some(w) = self.win.window.clone() else {
            return;
        };
        if !self.headless {
            w.set_minimized(false);
            w.focus_window();
            #[cfg(target_os = "macos")]
            crate::status_item::activate();
        }
        w.request_redraw();
    }
}

/// Text from a link as a toast shows it: no control characters, at most
/// 120 characters (a page writes it).
fn shown(text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    match clean.char_indices().nth(120) {
        Some((i, _)) => format!("{}…", &clean[..i]),
        None => clean,
    }
}
