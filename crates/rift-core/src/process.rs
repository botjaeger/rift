//! Bounded background tools: the deadline includes input and inherited pipes.

use std::io;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
fn nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
    // SAFETY: the caller owns this open pipe; preserve its existing flags.
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn drain(pipe: &mut Option<impl Read>, out: &mut Vec<u8>, max: usize) -> io::Result<bool> {
    let Some(reader) = pipe else { return Ok(false) };
    let mut changed = false;
    // Bound each turn too: a continuously writing child must not evade the deadline.
    for _ in 0..16 {
        let mut buf = [0; 16 * 1024];
        match reader.read(&mut buf) {
            Ok(0) => {
                *pipe = None;
                return Ok(true);
            }
            Ok(n) => {
                changed = true;
                out.extend_from_slice(&buf[..n.min(max.saturating_sub(out.len()))]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(changed)
}

/// Run a tool without a terminal, retaining at most `max` bytes per stream.
/// Drain excess bytes so a full pipe cannot deadlock it. Timeout stops its
/// private process group; no pipe-reader threads survive the call.
#[cfg(unix)]
pub(crate) fn output(
    command: &mut Command,
    input: Option<&[u8]>,
    limit: Duration,
    max: usize,
) -> io::Result<Output> {
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let deadline = Instant::now() + limit;
    let mut status = None;
    let result = (|| {
        if let Some(p) = &stdin {
            nonblocking(p)?;
        }
        if let Some(p) = &stdout {
            nonblocking(p)?;
        }
        if let Some(p) = &stderr {
            nonblocking(p)?;
        }
        let input = input.unwrap_or_default();
        let mut written = 0;
        let mut out = Vec::new();
        let mut err = Vec::new();
        loop {
            let mut changed = drain(&mut stdout, &mut out, max)?;
            changed |= drain(&mut stderr, &mut err, max)?;
            if let Some(pipe) = &mut stdin {
                if written == input.len() {
                    stdin = None;
                    changed = true;
                } else {
                    match pipe.write(&input[written..]) {
                        Ok(n) => {
                            written += n;
                            changed |= n > 0;
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                            stdin = None;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            // Keep the leader unreaped until its pipes close. A descendant
            // may leave the group while retaining a pipe; only the unreaped
            // leader pins the group ID for safe timeout cleanup.
            if status.is_none() && stdout.is_none() && stderr.is_none() {
                status = child.try_wait()?;
            }
            if let Some(status) = status
                && stdout.is_none()
                && stderr.is_none()
            {
                return Ok(Output {
                    status,
                    stdout: out,
                    stderr: err,
                });
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "tool timed out"));
            }
            if !changed {
                std::thread::sleep(
                    Duration::from_millis(10)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    })();
    if result.is_err() {
        if let Ok(pid) = libc::pid_t::try_from(child.id()) {
            // SAFETY: process_group(0) created this child's private group.
            // The leader is unreaped on every error path, pinning its ID;
            // never signal the caller's process group.
            unsafe { libc::killpg(pid, libc::SIGKILL) };
            // A hook being forked can miss the first signal on macOS.
            // Repeat only while the leader remains unreaped, pinning its PID.
            if status.is_none() {
                for _ in 0..5 {
                    std::thread::sleep(Duration::from_millis(10));
                    // SAFETY: the same private group, with its PID still pinned.
                    if unsafe { libc::killpg(pid, libc::SIGKILL) } != 0 {
                        break;
                    }
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Windows: anonymous pipes cannot be non-blocking, so each stream gets a
/// reader thread. On a timeout the process tree is killed; that closes the
/// pipes and ends the threads (a thread on a pipe some escaped process still
/// holds ends when that process does — it is never joined).
#[cfg(windows)]
pub(crate) fn output(
    command: &mut Command,
    input: Option<&[u8]>,
    limit: Duration,
    max: usize,
) -> io::Result<Output> {
    use std::io::{Read, Write};
    use std::sync::mpsc;
    fn reader(mut pipe: impl Read + Send + 'static, max: usize) -> mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut buf = [0; 16 * 1024];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => out.extend_from_slice(&buf[..n.min(max.saturating_sub(out.len()))]),
                }
            }
            let _ = tx.send(out);
        });
        rx
    }
    let mut child = crate::sys::own_group(command)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let (Some(mut pipe), Some(input)) = (child.stdin.take(), input) {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&input);
        });
    }
    let out = child.stdout.take().map(|p| reader(p, max));
    let err = child.stderr.take().map(|p| reader(p, max));
    let deadline = Instant::now() + limit;
    let collect = |rx: &Option<mpsc::Receiver<Vec<u8>>>| -> Option<Vec<u8>> {
        let rx = rx.as_ref()?;
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()
    };
    let result = (|| {
        let stdout = collect(&out);
        let stderr = collect(&err);
        loop {
            if let Some(status) = child.try_wait()? {
                if let (Some(stdout), Some(stderr)) = (stdout, stderr) {
                    return Ok(Output {
                        status,
                        stdout,
                        stderr,
                    });
                }
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "tool timed out"))
    })();
    if result.is_err() {
        crate::sys::kill_group(&child);
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn drains_large_output_while_writing_input_and_caps_capture() {
        let input = vec![b'x'; 256 * 1024];
        let out = output(
            Command::new("/bin/sh").args(["-c", "cat; printf error >&2"]),
            Some(&input),
            Duration::from_secs(5),
            1024,
        )
        .unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, vec![b'x'; 1024]);
        assert_eq!(out.stderr, b"error");
    }

    #[test]
    fn blocked_input_obeys_deadline() {
        let start = Instant::now();
        let error = output(
            Command::new("/bin/sleep").arg("2"),
            Some(&vec![b'x'; 256 * 1024]),
            Duration::from_millis(100),
            1024,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn inherited_pipes_do_not_extend_the_deadline() {
        let start = Instant::now();
        let error = output(
            Command::new("/bin/sh").args(["-c", "sleep 2 & exit 0"]),
            None,
            Duration::from_millis(100),
            1024,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
