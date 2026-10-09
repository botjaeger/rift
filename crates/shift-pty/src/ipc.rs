//! Wire protocol between Shift windows and the session service
//! (`shift-terminal --session-daemon`), over a per-user Unix socket.
//!
//! Frames: `u32` big-endian length, then a kind byte, then the body.
//!
//! | kind | body |
//! |---|---|
//! | [`FRAME_JSON`] | one [`Request`] (client → service) or [`Event`] (service → client) as JSON |
//! | [`FRAME_OUTPUT`] | `u64` session, `u64` first byte's sequence number, `u64` wall-clock ms, output bytes |
//! | [`FRAME_INPUT`] | `u64` session, bytes for the PTY |
//!
//! Output sequence numbers count bytes since the session started, so a
//! client that reconnects asks for everything after the last byte it saw.
//! Frames larger than [`MAX_FRAME`] are rejected (the connection closes).

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 4 << 20;
pub const FRAME_JSON: u8 = 0;
pub const FRAME_OUTPUT: u8 = 1;
pub const FRAME_INPUT: u8 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello {
        version: u32,
        /// The window's process id (listed as an attached client).
        pid: u32,
    },
    /// Start a shell; the reply is `Spawned`, then output as for `Attach`.
    Spawn {
        shell: Option<String>,
        cwd: Option<String>,
        env: Vec<(String, String)>,
        cols: u16,
        rows: u16,
        cell_w: u16,
        cell_h: u16,
    },
    /// Stream output after byte `from_seq` (replaying what is retained).
    Attach {
        session: u64,
        from_seq: u64,
    },
    Resize {
        session: u64,
        cols: u16,
        rows: u16,
        cell_w: u16,
        cell_h: u16,
    },
    /// Forget retained output (the window cleared its scrollback).
    ClearHistory {
        session: u64,
    },
    /// End the shell (the pane was closed on purpose).
    Kill {
        session: u64,
    },
    List,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: u64,
    pub pid: u32,
    pub cwd: Option<String>,
    /// Foreground process other than the shell, if any.
    pub job: Option<String>,
    /// Unix seconds.
    pub started: u64,
    /// Windows (process ids) currently attached.
    pub clients: Vec<u32>,
    pub retained_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Hello {
        version: u32,
        pid: u32,
    },
    Spawned {
        session: u64,
    },
    /// Replay begins at `from_seq` (later than requested when older output
    /// was no longer retained: `truncated`).
    Attached {
        session: u64,
        from_seq: u64,
        truncated: bool,
    },
    /// Everything retained has been sent; later output is live.
    ReplayDone {
        session: u64,
    },
    Status {
        session: u64,
        job: Option<String>,
    },
    Exited {
        session: u64,
    },
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    Error {
        message: String,
    },
}

/// A decoded frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Json(Vec<u8>),
    Output {
        session: u64,
        seq: u64,
        at_ms: u64,
        bytes: Vec<u8>,
    },
    Input {
        session: u64,
        bytes: Vec<u8>,
    },
}

fn write_frame(w: &mut impl Write, kind: u8, parts: &[&[u8]]) -> std::io::Result<()> {
    let len: usize = 1 + parts.iter().map(|p| p.len()).sum::<usize>();
    if len > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut buf = Vec::with_capacity(4 + len);
    buf.extend_from_slice(&(len as u32).to_be_bytes());
    buf.push(kind);
    for p in parts {
        buf.extend_from_slice(p);
    }
    w.write_all(&buf)
}

pub fn send_json(w: &mut impl Write, msg: &impl Serialize) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    write_frame(w, FRAME_JSON, &[&body])
}

pub fn send_output(
    w: &mut impl Write,
    session: u64,
    seq: u64,
    at_ms: u64,
    bytes: &[u8],
) -> std::io::Result<()> {
    write_frame(
        w,
        FRAME_OUTPUT,
        &[
            &session.to_be_bytes(),
            &seq.to_be_bytes(),
            &at_ms.to_be_bytes(),
            bytes,
        ],
    )
}

/// Input bytes per frame: large pastes are split to stay under the frame
/// limit.
pub const MAX_INPUT_CHUNK: usize = MAX_FRAME / 2;

pub fn send_input(w: &mut impl Write, session: u64, bytes: &[u8]) -> std::io::Result<()> {
    for chunk in bytes.chunks(MAX_INPUT_CHUNK) {
        write_frame(w, FRAME_INPUT, &[&session.to_be_bytes(), chunk])?;
    }
    Ok(())
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// Read one frame. `Ok(None)` on a clean end of stream.
pub fn read_frame(r: &mut impl Read) -> std::io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(std::io::Error::other("bad frame length"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let bad = || std::io::Error::other("truncated frame");
    Ok(Some(match body[0] {
        FRAME_JSON => Frame::Json(body[1..].to_vec()),
        FRAME_OUTPUT => Frame::Output {
            session: u64_at(&body, 1).ok_or_else(bad)?,
            seq: u64_at(&body, 9).ok_or_else(bad)?,
            at_ms: u64_at(&body, 17).ok_or_else(bad)?,
            bytes: body.get(25..).ok_or_else(bad)?.to_vec(),
        },
        FRAME_INPUT => Frame::Input {
            session: u64_at(&body, 1).ok_or_else(bad)?,
            bytes: body.get(9..).ok_or_else(bad)?.to_vec(),
        },
        _ => return Err(std::io::Error::other("unknown frame kind")),
    }))
}

/// The service's socket: `<data dir>/sessiond/sock`, in a 0700 directory.
pub fn socket_path() -> Option<std::path::PathBuf> {
    socket_dir("sessiond").map(|d| d.join("sock"))
}

/// `<data dir>/<name>` for Unix sockets, or a short, data-dir-specific path
/// under `/tmp` when the original would exceed the Unix socket path limit.
/// Created 0700 by the caller.
pub fn socket_dir(name: &str) -> Option<std::path::PathBuf> {
    let data_dir = shift_core::shell_integration::data_dir()?;
    let uid = unsafe { libc::getuid() };
    Some(socket_dir_for(&data_dir, name, uid))
}

fn socket_dir_for(data_dir: &std::path::Path, name: &str, uid: u32) -> std::path::PathBuf {
    let dir = data_dir.join(name);
    // Leave room for "/<pid>.sock" and keep separate XDG data dirs isolated.
    if dir.as_os_str().len() + 20 < 100 {
        return dir;
    }
    use std::os::unix::ffi::OsStrExt;
    let hash = data_dir
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    std::path::Path::new("/tmp")
        .join(format!("shift-{uid}-{hash:016x}"))
        .join(name)
}

/// Uid of the process at the other end of a Unix socket.
pub fn peer_uid(s: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let fd = s.as_raw_fd();
    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd"))]
    {
        let (mut uid, mut gid) = (0, 0);
        (unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } == 0).then_some(uid)
    }
    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let ok = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        } == 0;
        ok.then_some(cred.uid)
    }
    // unknown platform: refuse (the socket directory is 0700 regardless)
    #[cfg(not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "linux"
    )))]
    {
        let _ = fd;
        None
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_fallback_is_scoped_to_data_dir() {
        let a = std::path::PathBuf::from(format!("/tmp/{}", "a".repeat(100)));
        let b = std::path::PathBuf::from(format!("/tmp/{}", "b".repeat(100)));
        let a_control = socket_dir_for(&a, "control", 501);
        let b_control = socket_dir_for(&b, "control", 501);
        assert_ne!(a_control, b_control);
        assert_eq!(
            a_control.parent(),
            socket_dir_for(&a, "sessiond", 501).parent()
        );
        assert!(a_control.as_os_str().len() + 20 < 100);
        assert_eq!(
            socket_dir_for(std::path::Path::new("/tmp/short"), "control", 501),
            std::path::Path::new("/tmp/short/control")
        );
    }

    #[test]
    fn frames_roundtrip() {
        let mut buf = Vec::new();
        send_json(
            &mut buf,
            &Request::Attach {
                session: 3,
                from_seq: 99,
            },
        )
        .unwrap();
        send_output(&mut buf, 3, 100, 42, b"hello").unwrap();
        send_input(&mut buf, 3, b"ls\r").unwrap();
        let mut r = std::io::Cursor::new(buf);
        let Frame::Json(j) = read_frame(&mut r).unwrap().unwrap() else {
            panic!()
        };
        assert_eq!(
            serde_json::from_slice::<Request>(&j).unwrap(),
            Request::Attach {
                session: 3,
                from_seq: 99
            }
        );
        assert_eq!(
            read_frame(&mut r).unwrap(),
            Some(Frame::Output {
                session: 3,
                seq: 100,
                at_ms: 42,
                bytes: b"hello".to_vec()
            })
        );
        assert_eq!(
            read_frame(&mut r).unwrap(),
            Some(Frame::Input {
                session: 3,
                bytes: b"ls\r".to_vec()
            })
        );
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    #[test]
    fn malformed_frames_are_errors_not_panics() {
        for bad in [
            &[0, 0, 0, 0][..],                     // zero length
            &[0xff, 0xff, 0xff, 0xff][..],         // over the limit
            &[0, 0, 0, 3, FRAME_OUTPUT, 1, 2][..], // truncated header
            &[0, 0, 0, 2, 9, 0][..],               // unknown kind
            &[0, 0, 0, 9][..],                     // body cut off
        ] {
            assert!(
                read_frame(&mut std::io::Cursor::new(bad)).is_err(),
                "{bad:?}"
            );
        }
        // unknown JSON is a parse error at the caller, not a framing error
        let mut buf = Vec::new();
        write_frame(&mut buf, FRAME_JSON, &[b"{\"type\":\"nope\"}"]).unwrap();
        let Frame::Json(j) = read_frame(&mut std::io::Cursor::new(buf)).unwrap().unwrap() else {
            panic!()
        };
        assert!(serde_json::from_slice::<Request>(&j).is_err());
    }
}
