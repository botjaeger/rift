//! Platform differences for the gateway and the relay (this crate has no
//! app dependencies, so it carries its own): socket polling, file modes and
//! file identity. Unix is the reference; Windows gets the nearest equivalent.

use std::fs::{Metadata, OpenOptions};
use std::io;
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
pub(crate) type PollFd = libc::pollfd;
#[cfg(windows)]
pub(crate) type PollFd = windows_sys::Win32::Networking::WinSock::WSAPOLLFD;

#[cfg(unix)]
pub(crate) type RawSock = std::os::fd::RawFd;
#[cfg(windows)]
pub(crate) type RawSock = usize;

#[cfg(unix)]
pub(crate) const POLLIN: i16 = libc::POLLIN;
#[cfg(unix)]
pub(crate) const POLLOUT: i16 = libc::POLLOUT;
#[cfg(windows)]
pub(crate) const POLLIN: i16 = windows_sys::Win32::Networking::WinSock::POLLRDNORM;
#[cfg(windows)]
pub(crate) const POLLOUT: i16 = windows_sys::Win32::Networking::WinSock::POLLWRNORM;

/// The socket under `s`, for [`poll_fds`].
pub(crate) fn raw(s: &TcpStream) -> RawSock {
    #[cfg(unix)]
    {
        std::os::fd::AsRawFd::as_raw_fd(s)
    }
    #[cfg(windows)]
    {
        std::os::windows::io::AsRawSocket::as_raw_socket(s) as usize
    }
}

pub(crate) fn pollfd(fd: RawSock, events: i16) -> PollFd {
    PollFd {
        fd,
        events,
        revents: 0,
    }
}

/// Wait until one of `fds` is ready or `timeout` passes.
pub(crate) fn poll_fds(fds: &mut [PollFd], timeout: Duration) {
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: `fds` is a valid array of poll entries for the duration of
    // the call
    #[cfg(unix)]
    unsafe {
        libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms);
    }
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::Networking::WinSock::WSAPoll(fds.as_mut_ptr(), fds.len() as u32, ms);
    }
}

/// Unix permission bits for files this opens. Windows files inherit the
/// folder's ACL (the user's profile folders are private to them).
pub(crate) trait UnixMode {
    fn unix_mode(&mut self, mode: u32) -> &mut Self;
}

impl UnixMode for OpenOptions {
    fn unix_mode(&mut self, mode: u32) -> &mut Self {
        #[cfg(unix)]
        {
            std::os::unix::fs::OpenOptionsExt::mode(self, mode)
        }
        #[cfg(windows)]
        {
            let _ = mode;
            self
        }
    }
}

/// `chmod`; a no-op on Windows (see [`UnixMode`]).
pub(crate) fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(windows)]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// `std::fs::rename` that replaces `to`, retried for up to half a second on
/// Windows while another process holds `to` open without delete sharing
/// (the same as `rift_core::sys::rename`; this crate has no app deps).
pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        loop {
            match std::fs::rename(from, to) {
                Err(e)
                    if e.kind() == io::ErrorKind::PermissionDenied
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                other => return other,
            }
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

/// The inode on Unix; 0 on Windows (callers also compare size and time).
pub(crate) fn file_id(meta: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        std::os::unix::fs::MetadataExt::ino(meta)
    }
    #[cfg(windows)]
    {
        let _ = meta;
        0
    }
}

/// A file to hold an exclusive lock on for `dir`: the directory itself on
/// Unix (`flock` works on it), a `.lock` file inside it on Windows (a
/// directory cannot be opened as a file there).
pub(crate) fn dir_lock_file(dir: &Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)
    }
    #[cfg(windows)]
    {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".lock"))
    }
}

/// This machine's IPv4 addresses a phone on the LAN can reach: every up,
/// non-loopback, non-link-local interface address.
#[cfg(unix)]
pub(crate) fn ipv4_addrs() -> Vec<std::net::Ipv4Addr> {
    use std::net::Ipv4Addr;
    let mut out = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a linked list we only read, then free once
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut cur = list;
        while let Some(ifa) = cur.as_ref() {
            let up = ifa.ifa_flags & libc::IFF_UP as libc::c_uint != 0;
            if up
                && let Some(sa) = ifa.ifa_addr.as_ref()
                && i32::from(sa.sa_family) == libc::AF_INET
            {
                let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                if !ip.is_loopback() && !ip.is_link_local() && !out.contains(&ip) {
                    out.push(ip);
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    out
}

/// Windows: the address the default route leaves from (a UDP "connect"
/// sends nothing; it only picks the route).
#[cfg(windows)]
pub(crate) fn ipv4_addrs() -> Vec<std::net::Ipv4Addr> {
    let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    if s.connect("192.0.2.1:9").is_err() {
        return Vec::new();
    }
    match s.local_addr().map(|a| a.ip()) {
        Ok(std::net::IpAddr::V4(ip))
            if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() =>
        {
            vec![ip]
        }
        _ => Vec::new(),
    }
}
