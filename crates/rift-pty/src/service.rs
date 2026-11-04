//! Client side of the session service: connect (starting the service when
//! it is not running), then spawn or reattach a shell.

use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use alacritty_terminal::event::WindowSize;

use crate::ipc::{self, Event, Frame, Request};
use crate::session::ServiceTarget;

/// Connect to the running service, or start it (`<this exe>
/// --session-daemon`, detached so it outlives the window: a launchd job on
/// macOS, see `launchd`).
pub fn connect() -> std::io::Result<UnixStream> {
    connect_features().map(|(s, _)| s)
}

/// `connect`, also returning the features the service listed in its hello.
fn connect_features() -> std::io::Result<(UnixStream, Vec<String>)> {
    let path = ipc::socket_path().ok_or_else(|| std::io::Error::other("no data dir"))?;
    if let Ok(s) = UnixStream::connect(&path) {
        return handshake_features(s);
    }
    // panes connect on their own threads: one starts the service, the
    // others wait for it (two starts race launchd's bootstrap/bootout)
    static START: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _start = START.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(s) = UnixStream::connect(&path) {
        return handshake_features(s);
    }
    let exe = std::env::current_exe()?;
    crate::launchd::spawn_helper("session", &exe, &["--session-daemon"], None)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(&path) {
            Ok(s) => return handshake_features(s),
            // a service that never came up counts as stalled: the window
            // stays local instead of waiting here for every new pane
            Err(e) if Instant::now() > deadline => {
                return Err(std::io::Error::new(
                    ErrorKind::TimedOut,
                    format!("the session service did not start ({e})"),
                ));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(30)),
        }
    }
}

fn handshake(s: UnixStream) -> std::io::Result<UnixStream> {
    handshake_features(s).map(|(s, _)| s)
}

/// `handshake`, also returning the features the service listed.
fn handshake_features(mut s: UnixStream) -> std::io::Result<(UnixStream, Vec<String>)> {
    // keystrokes go to this socket: only to a service run by this user
    if ipc::peer_uid(&s) != Some(unsafe { libc::getuid() }) {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "session service socket belongs to another user",
        ));
    }
    ipc::send_json(
        &mut s,
        &Request::Hello {
            version: ipc::PROTOCOL_VERSION,
            pid: std::process::id(),
        },
    )?;
    match next_event(&mut s)? {
        Event::Hello {
            version, features, ..
        } if version == ipc::PROTOCOL_VERSION => Ok((s, features)),
        Event::Hello { version, .. } => Err(std::io::Error::other(format!(
            "session service speaks protocol {version}, this build {}",
            ipc::PROTOCOL_VERSION
        ))),
        other => Err(std::io::Error::other(format!("unexpected {other:?}"))),
    }
}

/// How long the service may take to answer one setup request (hello, spawn,
/// attach, list) in total. A service that accepts but stalls then fails with
/// `TimedOut` and the window falls back to a local shell.
pub const SETUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Reads from a socket until a fixed deadline: each read waits only for
/// what is left, so a peer dripping bytes cannot extend it.
struct Deadline<'a> {
    s: &'a UnixStream,
    until: Instant,
}

impl std::io::Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        self.s.set_read_timeout(Some(left))?;
        match std::io::Read::read(&mut &*self.s, buf) {
            // SO_RCVTIMEO expiry reads as WouldBlock on unix
            Err(e) if e.kind() == ErrorKind::WouldBlock => Err(ErrorKind::TimedOut.into()),
            r => r,
        }
    }
}

/// Read until the next JSON event (output frames are not expected here),
/// within [`SETUP_TIMEOUT`]. The stream is blocking again afterwards: the
/// pane's reader owns it next.
pub fn next_event(s: &mut UnixStream) -> std::io::Result<Event> {
    next_event_within(s, SETUP_TIMEOUT)
}

fn next_event_within(s: &mut UnixStream, limit: Duration) -> std::io::Result<Event> {
    let mut r = Deadline {
        s,
        until: Instant::now() + limit,
    };
    let event = loop {
        match ipc::read_frame(&mut r) {
            Ok(Some(Frame::Json(body))) => {
                break serde_json::from_slice(&body).map_err(std::io::Error::other);
            }
            Ok(Some(_)) => continue,
            Ok(None) => break Err(std::io::Error::from(ErrorKind::UnexpectedEof)),
            Err(e) => break Err(e),
        }
    };
    s.set_read_timeout(None)?;
    event
}

/// A service from before profiles would start the shell without its
/// arguments: refuse a spawn there, and the window starts this one locally
/// with them. A reattach sends no arguments (the shell runs already).
fn refuses_args(target: ServiceTarget, args: &[String], features: &[String]) -> bool {
    target == ServiceTarget::Spawn
        && !args.is_empty()
        && !features.iter().any(|f| f == ipc::FEATURE_ARGS)
}

/// Spawn or reattach; the returned stream next delivers the replay, then
/// `ReplayDone`, then live output. Returns the service's session id and,
/// for a reattach, the directory the service last saw the shell in.
pub fn open(
    target: ServiceTarget,
    shell: Option<&str>,
    args: Vec<String>,
    cwd: Option<&std::path::Path>,
    env: Vec<(String, String)>,
    ws: WindowSize,
) -> std::io::Result<(UnixStream, u64, Option<String>)> {
    let (mut s, features) = connect_features()?;
    if refuses_args(target, &args, &features) {
        return Err(std::io::Error::new(
            ErrorKind::Unsupported,
            "the running session service predates profile arguments",
        ));
    }
    let session = match target {
        ServiceTarget::Spawn => {
            ipc::send_json(
                &mut s,
                &Request::Spawn {
                    shell: shell.map(str::to_string),
                    args,
                    cwd: cwd.map(|p| p.to_string_lossy().into_owned()),
                    env,
                    cols: ws.num_cols,
                    rows: ws.num_lines,
                    cell_w: ws.cell_width,
                    cell_h: ws.cell_height,
                },
            )?;
            match next_event(&mut s)? {
                Event::Spawned { session } => session,
                Event::Error { message } => return Err(std::io::Error::other(message)),
                other => return Err(std::io::Error::other(format!("unexpected {other:?}"))),
            }
        }
        ServiceTarget::Attach(id) => id,
    };
    // a reattached shell's last OSC 7 may have left the replay ring; the
    // service still knows its directory. Asked before attaching: once the
    // replay is queued to this window, a list waits on that shell's replay
    // lock, and a service that also holds its shell table meanwhile (every
    // release up to 0.22) stalls every other pane's spawn and attach
    let known_cwd = match target {
        ServiceTarget::Attach(id) => {
            ipc::send_json(&mut s, &Request::List)?;
            match next_event(&mut s)? {
                Event::Sessions { sessions } => sessions
                    .into_iter()
                    .find(|i| i.id == id)
                    .and_then(|i| i.cwd),
                other => return Err(std::io::Error::other(format!("unexpected {other:?}"))),
            }
        }
        ServiceTarget::Spawn => None,
    };
    let attached = attach_new(&mut s, session, target, ws);
    if attached.is_err() && target == ServiceTarget::Spawn {
        // nobody would ever reach the shell just started
        kill(session);
    }
    attached.map(|()| (s, session, known_cwd))
}

/// Attach to `session` just spawned or to reattach, then send the size.
fn attach_new(
    s: &mut UnixStream,
    session: u64,
    target: ServiceTarget,
    ws: WindowSize,
) -> std::io::Result<()> {
    ipc::send_json(
        s,
        &Request::Attach {
            session,
            from_seq: 0,
        },
    )?;
    match next_event(s)? {
        Event::Attached { .. } => {}
        Event::Error { message } => {
            return Err(std::io::Error::new(ErrorKind::NotFound, message));
        }
        other => return Err(std::io::Error::other(format!("unexpected {other:?}"))),
    }
    if matches!(target, ServiceTarget::Attach(_)) {
        // the window size may have changed while detached
        ipc::send_json(
            s,
            &Request::Resize {
                session,
                cols: ws.num_cols,
                rows: ws.num_lines,
                cell_w: ws.cell_width,
                cell_h: ws.cell_height,
            },
        )?;
    }
    Ok(())
}

/// Reattach to `session` on a fresh connection after the service dropped
/// this one (it does so to a client that falls too far behind). Streams the
/// output after byte `from_seq`, then `ReplayDone`, then live output; true
/// when output after `from_seq` was no longer retained (the replay starts
/// later). Never starts the service: if it is gone, so is the shell.
pub fn resume(session: u64, from_seq: u64) -> std::io::Result<(UnixStream, bool)> {
    let path = ipc::socket_path().ok_or_else(|| std::io::Error::other("no data dir"))?;
    let mut s = handshake(UnixStream::connect(path)?)?;
    ipc::send_json(&mut s, &Request::Attach { session, from_seq })?;
    match next_event(&mut s)? {
        Event::Attached { truncated, .. } => Ok((s, truncated)),
        Event::Error { message } => Err(std::io::Error::new(ErrorKind::NotFound, message)),
        other => Err(std::io::Error::other(format!("unexpected {other:?}"))),
    }
}

/// End `session`'s shell (a pane closed on purpose), on a connection of its
/// own. Never starts the service: if it is gone, so is the shell.
pub fn kill(session: u64) {
    let Some(path) = ipc::socket_path() else {
        return;
    };
    let Ok(Ok(mut s)) = UnixStream::connect(path).map(handshake) else {
        return;
    };
    let _ = ipc::send_json(&mut s, &Request::Kill { session });
}

/// Sessions the service is running (empty when it is not running; never
/// starts it).
pub fn list() -> Vec<ipc::SessionInfo> {
    let Some(path) = ipc::socket_path() else {
        return Vec::new();
    };
    let Ok(s) = UnixStream::connect(path) else {
        return Vec::new();
    };
    let Ok(mut s) = handshake(s) else {
        return Vec::new();
    };
    if ipc::send_json(&mut s, &Request::List).is_err() {
        return Vec::new();
    }
    match next_event(&mut s) {
        Ok(Event::Sessions { sessions }) => sessions,
        _ => Vec::new(),
    }
}

/// What the running session service says about the privacy switches of the
/// app it runs as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePrivacy {
    /// It answered: Full Disk Access (`None`: it cannot tell).
    Answer { full_disk: Option<bool> },
    /// It runs, from a release that cannot be asked (restart Rift).
    Outdated,
    /// None runs, or it did not answer in time.
    Unreachable,
}

/// How long the service may take to answer a privacy question.
const PRIVACY_TIMEOUT: Duration = Duration::from_secs(1);

/// Ask the service about its own privacy grants. Never starts it. Blocks up
/// to [`SETUP_TIMEOUT`] on the hello of a stalled service: call it off the
/// UI thread.
pub fn privacy() -> ServicePrivacy {
    let Some(path) = ipc::socket_path() else {
        return ServicePrivacy::Unreachable;
    };
    let Ok(s) = UnixStream::connect(path) else {
        return ServicePrivacy::Unreachable;
    };
    let Ok((mut s, features)) = handshake_features(s) else {
        return ServicePrivacy::Unreachable;
    };
    ask_privacy(&mut s, &features)
}

/// `privacy` on a connection that has said hello.
fn ask_privacy(s: &mut UnixStream, features: &[String]) -> ServicePrivacy {
    if !features.iter().any(|f| f == ipc::FEATURE_PRIVACY) {
        return ServicePrivacy::Outdated;
    }
    if ipc::send_json(s, &Request::Privacy).is_err() {
        return ServicePrivacy::Unreachable;
    }
    match next_event_within(s, PRIVACY_TIMEOUT) {
        Ok(Event::Privacy { full_disk }) => ServicePrivacy::Answer { full_disk },
        _ => ServicePrivacy::Unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn a_service_without_the_feature_is_outdated_and_is_not_asked() {
        let (mut ours, mut theirs) = UnixStream::pair().unwrap();
        assert_eq!(ask_privacy(&mut ours, &[]), ServicePrivacy::Outdated);
        // nothing was sent to it
        theirs
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut b = [0u8; 1];
        assert!(std::io::Read::read(&mut theirs, &mut b).is_err());
    }

    #[test]
    fn the_answer_of_a_service_is_passed_on_and_a_silent_one_is_unreachable() {
        let feature = vec![ipc::FEATURE_PRIVACY.to_string()];
        let (mut ours, mut theirs) = UnixStream::pair().unwrap();
        ipc::send_json(
            &mut theirs,
            &Event::Privacy {
                full_disk: Some(false),
            },
        )
        .unwrap();
        assert_eq!(
            ask_privacy(&mut ours, &feature),
            ServicePrivacy::Answer {
                full_disk: Some(false)
            }
        );
        let (mut ours, _theirs) = UnixStream::pair().unwrap();
        let t = Instant::now();
        assert_eq!(
            ask_privacy(&mut ours, &feature),
            ServicePrivacy::Unreachable
        );
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    }

    fn hello_frame() -> Vec<u8> {
        let mut b = Vec::new();
        ipc::send_json(
            &mut b,
            &Event::Hello {
                version: ipc::PROTOCOL_VERSION,
                pid: 1,
                features: vec![],
            },
        )
        .unwrap();
        b
    }

    #[test]
    fn only_a_spawn_with_arguments_needs_the_args_feature() {
        let args = vec!["-l".to_string()];
        let old: Vec<String> = vec![];
        let new = vec![ipc::FEATURE_ARGS.to_string()];
        assert!(refuses_args(ServiceTarget::Spawn, &args, &old));
        assert!(!refuses_args(ServiceTarget::Spawn, &args, &new));
        assert!(!refuses_args(ServiceTarget::Spawn, &[], &old));
        // a reattach sends no arguments: an old service can serve it
        assert!(!refuses_args(ServiceTarget::Attach(7), &args, &old));
    }

    #[test]
    fn a_silent_service_times_out() {
        let (mut ours, _theirs) = UnixStream::pair().unwrap();
        let t = Instant::now();
        let e = next_event_within(&mut ours, Duration::from_millis(300)).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        // handed back blocking: the pane's reader must never time out
        assert_eq!(ours.read_timeout().unwrap(), None);
    }

    /// One byte at a time, each sooner than the limit: the total still ends.
    #[test]
    fn a_dripping_service_cannot_extend_the_deadline() {
        let (mut ours, mut theirs) = UnixStream::pair().unwrap();
        let frame = hello_frame();
        let drip = std::thread::spawn(move || {
            for b in frame {
                std::thread::sleep(Duration::from_millis(60));
                if theirs.write_all(&[b]).is_err() {
                    break;
                }
            }
        });
        let t = Instant::now();
        let e = next_event_within(&mut ours, Duration::from_millis(400)).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        drop(ours);
        drip.join().unwrap();
    }

    #[test]
    fn a_prompt_answer_is_read_and_the_stream_stays_blocking() {
        let (mut ours, mut theirs) = UnixStream::pair().unwrap();
        theirs.write_all(&hello_frame()).unwrap();
        let ev = next_event_within(&mut ours, Duration::from_millis(300)).unwrap();
        assert!(matches!(ev, Event::Hello { .. }));
        assert_eq!(ours.read_timeout().unwrap(), None);
    }
}
