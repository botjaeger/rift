//! Shells a paired phone starts while no Rift window is open
//! (`remote_when_closed`): the gateway keeps them in the session service and
//! serves them as one background window (pid 0) until Rift opens, which then
//! shows them as tabs (`adopt`).
//!
//! The gateway attaches to each shell like a window does — a
//! [`rift_pty::Session`] parsing the service's output into a grid and blocks
//! — so the phone reads them through the same helpers as a window's panes.
//! Their service ids are listed in `<data dir>/background-shells.json`; a
//! window starting up takes that list (and removes it), and the gateway lets
//! go of its sessions (the shells keep running in the service) as soon as a
//! window is open.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rift_pty::{GridSize, Session, SessionEvent, SpawnOptions};
use rift_remote::protocol::{OpenKind, Pane, Tab, Window};
use rift_remote::{InputError, Key};
use serde::{Deserialize, Serialize};

/// The most shells a phone may keep while Rift is closed.
pub const MAX_SHELLS: usize = 8;
/// The grid the gateway gives a background shell (a phone's terminal view
/// reads its screen; Rift resizes it when it adopts the shell).
const SIZE: GridSize = GridSize {
    cols: 100,
    rows: 32,
};

/// One background shell.
struct Shell {
    /// Its pane id for phones (pid 0, so the id itself).
    id: u64,
    session: Session,
    /// The shell exited (the reader thread said so).
    exited: Arc<AtomicBool>,
}

/// The gateway's background shells.
#[derive(Default)]
pub struct Background {
    shells: Mutex<Vec<Shell>>,
    next: Mutex<u64>,
}

/// `background-shells.json`: the shells for the next window to adopt.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Handoff {
    /// Session service ids, oldest first.
    pub sessions: Vec<u64>,
}

fn handoff_path() -> Option<PathBuf> {
    rift_core::shell_integration::data_dir().map(|d| d.join("background-shells.json"))
}

/// The shells left for this window, removing the list (a window starting
/// up). Unreadable or invalid lists are dropped.
pub fn adopt() -> Vec<u64> {
    let Some(path) = handoff_path() else {
        return Vec::new();
    };
    let list = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Handoff>(&b).ok())
        .map(|h| h.sessions)
        .unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    list.into_iter().take(MAX_SHELLS).collect()
}

fn write_handoff(sessions: Vec<u64>) {
    let Some(path) = handoff_path() else { return };
    if sessions.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Ok(json) = serde_json::to_vec(&Handoff { sessions }) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

impl Background {
    /// Start a shell in the session service, in the home folder. `Err` when
    /// the computer refuses (`remote_when_closed` or `durable_sessions` off,
    /// too many shells) or the service cannot start one.
    pub fn start(&self) -> Result<u64, InputError> {
        let config = rift_core::Config::try_load()
            .ok()
            .flatten()
            .unwrap_or_default();
        if !(config.remote_access && config.remote_when_closed && config.durable_sessions) {
            return Err(InputError::Denied);
        }
        self.prune();
        if self.shells.lock().unwrap().len() >= MAX_SHELLS {
            return Err(InputError::Unavailable(format!(
                "at most {MAX_SHELLS} shells while Rift is closed"
            )));
        }
        let id = {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            *next
        };
        let exited = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let observer: rift_pty::Observer = Arc::new({
            let (exited, failed) = (exited.clone(), failed.clone());
            move |_, event| match event {
                SessionEvent::Exit => exited.store(true, Ordering::Release),
                SessionEvent::ServiceFailed(_) => failed.store(true, Ordering::Release),
                _ => {}
            }
        });
        let session = Session::spawn(SpawnOptions {
            id,
            size: SIZE,
            cell_size: (8, 16),
            shell: config.shell.clone(),
            working_directory: std::env::var_os("HOME").map(PathBuf::from),
            env: vec![],
            scrollback: config.scrollback_lines(),
            observer,
            replay: None,
            service: Some(rift_pty::ServiceTarget::Spawn),
            clipboard: rift_pty::ClipboardAccess::from_config(&config.clipboard_access),
        })
        .map_err(|e| InputError::Unavailable(format!("the session service: {e}")))?;
        // the service answers on the pane's reader thread; this is the
        // gateway's, which may wait for it (bounded by the setup deadline)
        let until = std::time::Instant::now() + rift_pty::service::SETUP_TIMEOUT * 2;
        while session.service_id().is_none()
            && !failed.load(Ordering::Acquire)
            && std::time::Instant::now() < until
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if session.service_id().is_none() {
            // never a shell only this gateway holds: Rift could not adopt it
            session.terminate();
            return Err(InputError::Unavailable(
                "the session service is not running".into(),
            ));
        }
        self.shells.lock().unwrap().push(Shell {
            id,
            session,
            exited,
        });
        self.save();
        Ok(id)
    }

    /// Forget shells that exited.
    fn prune(&self) {
        let before = self.shells.lock().unwrap().len();
        self.shells
            .lock()
            .unwrap()
            .retain(|s| !s.exited.load(Ordering::Acquire));
        if self.shells.lock().unwrap().len() != before {
            self.save();
        }
    }

    fn save(&self) {
        let ids = self
            .shells
            .lock()
            .unwrap()
            .iter()
            .filter_map(|s| s.session.service_id())
            .collect();
        write_handoff(ids);
    }

    /// A window is open: let go of every shell (they keep running in the
    /// service; the window adopted them, or will from the list).
    pub fn release(&self) {
        let released = std::mem::take(&mut *self.shells.lock().unwrap());
        if !released.is_empty() {
            log::info!(
                "remote: {} background shell(s) handed to Rift",
                released.len()
            );
        }
    }

    /// The background window, when there are shells.
    pub fn window(&self) -> Option<Window> {
        self.prune();
        let shells = self.shells.lock().unwrap();
        if shells.is_empty() {
            return None;
        }
        let last = shells.len() - 1;
        let tabs = shells
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let recent = s
                    .session
                    .blocks
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|b| !b.command.is_empty())
                    .map(|b| (b.command.clone(), b.exit_code()));
                let job = s.session.foreground_job_name();
                Tab {
                    index: i,
                    title: job.clone().unwrap_or_else(|| "zsh".into()),
                    active: i == last,
                    panes: vec![Pane {
                        id: s.id,
                        active: true,
                        cwd: s.session.cwd.lock().unwrap().clone(),
                        job,
                        last_command: recent.as_ref().map(|r| r.0.clone()),
                        last_exit: recent.and_then(|r| r.1),
                        git: None,
                        chips: None,
                        waiting: None,
                        running: crate::views::terminal::remote_running(&s.session),
                    }],
                }
            })
            .collect();
        Some(Window {
            pid: 0,
            background: true,
            tabs,
        })
    }

    /// Run `f` on the shell of pane `id`; `None` when there is none.
    pub fn with<T>(&self, id: u64, f: impl FnOnce(&Session) -> T) -> Option<T> {
        let shells = self.shells.lock().unwrap();
        shells.iter().find(|s| s.id == id).map(|s| f(&s.session))
    }

    /// A phone's input for a background shell: there is no command line
    /// editor here, so a command is typed into the shell's own and Enter
    /// pressed; keys and text go to the shell as typed.
    pub fn input(&self, id: u64, input: rift_remote::Input) -> Result<(), InputError> {
        use crate::remote::Route;
        self.with(id, |s| {
            let route = {
                use rift_pty::alacritty_terminal::term::TermMode;
                let alt = s.term.lock().mode().contains(TermMode::ALT_SCREEN);
                let blocks = s.blocks.lock().unwrap();
                crate::remote::route(
                    alt,
                    blocks.last().map(|b| &b.state),
                    blocks.submit_pending(),
                )
            };
            match input {
                rift_remote::Input::Command(cmd) => {
                    if route != Route::Editor {
                        return Err(InputError::Busy);
                    }
                    let mut bytes = cmd.into_bytes();
                    bytes.push(b'\r');
                    s.write(&bytes);
                }
                rift_remote::Input::Keys(keys) => s.write(&key_bytes(s, &keys)),
                rift_remote::Input::Text(text) => {
                    if !route.takes_text() {
                        return Err(InputError::Busy);
                    }
                    s.write(text.as_bytes());
                }
            }
            Ok(())
        })
        .unwrap_or(Err(InputError::NoPane))
    }

    /// `close` of a background shell (its tab holds only it): the shell
    /// ends in the service and leaves the hand-off list.
    pub fn close(&self, id: u64) -> Result<(), InputError> {
        let shell = {
            let mut shells = self.shells.lock().unwrap();
            let at = shells.iter().position(|s| s.id == id);
            at.map(|i| shells.remove(i))
        };
        let Some(shell) = shell else {
            return Err(InputError::NoPane);
        };
        shell.session.terminate();
        self.save();
        Ok(())
    }

    /// `open` next to a background shell: another background tab.
    pub fn open(&self, id: u64, kind: OpenKind) -> Result<u64, InputError> {
        if self.with(id, |_| ()).is_none() {
            return Err(InputError::NoPane);
        }
        if kind == OpenKind::Pane {
            return Err(InputError::Unavailable(
                "split panes need Rift open; this opens a tab".into(),
            ));
        }
        self.start()
    }
}

/// The bytes `keys` produce in `session`'s program, as the desktop's keys
/// would.
fn key_bytes(session: &Session, keys: &[Key]) -> Vec<u8> {
    use rift_pty::alacritty_terminal::term::TermMode;
    let mode = *session.term.lock().mode();
    let enc = crate::input::KeyEncoding {
        app_cursor: mode.contains(TermMode::APP_CURSOR),
        option_as_meta: true,
        kitty_flags: crate::keyboard::flags_from_mode(mode),
    };
    let presses: Vec<_> = keys
        .iter()
        .map(|k| crate::input::remote_key_press(*k))
        .collect();
    crate::input::remote_bytes(&presses, enc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handoff_list_round_trips_and_bounds() {
        let h = Handoff {
            sessions: vec![3, 9],
        };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, r#"{"sessions":[3,9]}"#);
        assert_eq!(serde_json::from_str::<Handoff>(&json).unwrap(), h);
    }
}
