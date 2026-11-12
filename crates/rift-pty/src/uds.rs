//! The session service's socket types. Unix-domain sockets on macOS and
//! Linux; on Windows the service is not available yet (panes run local
//! PTYs), so the types exist but can never be connected: every constructor
//! fails with `Unsupported` and the remaining methods are unreachable.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(windows)]
pub use windows::{UnixListener, UnixStream};

#[cfg(windows)]
mod windows {
    use std::io;
    use std::path::Path;
    use std::time::Duration;

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "the session service is not available on Windows yet",
        )
    }

    /// Never constructed (see the module doc).
    #[derive(Debug)]
    pub enum UnixStream {}

    impl UnixStream {
        pub fn connect<P: AsRef<Path>>(_: P) -> io::Result<UnixStream> {
            Err(unsupported())
        }
        pub fn pair() -> io::Result<(UnixStream, UnixStream)> {
            Err(unsupported())
        }
        pub fn try_clone(&self) -> io::Result<UnixStream> {
            match *self {}
        }
        pub fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
            match *self {}
        }
        pub fn set_write_timeout(&self, _: Option<Duration>) -> io::Result<()> {
            match *self {}
        }
        pub fn set_nonblocking(&self, _: bool) -> io::Result<()> {
            match *self {}
        }
        pub fn shutdown(&self, _: std::net::Shutdown) -> io::Result<()> {
            match *self {}
        }
    }

    impl io::Read for UnixStream {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            match *self {}
        }
    }

    impl io::Write for UnixStream {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            match *self {}
        }
        fn flush(&mut self) -> io::Result<()> {
            match *self {}
        }
    }

    impl io::Read for &UnixStream {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            match **self {}
        }
    }

    impl io::Write for &UnixStream {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            match **self {}
        }
        fn flush(&mut self) -> io::Result<()> {
            match **self {}
        }
    }

    /// Never constructed (see the module doc).
    #[derive(Debug)]
    pub enum UnixListener {}

    impl UnixListener {
        pub fn bind<P: AsRef<Path>>(_: P) -> io::Result<UnixListener> {
            Err(unsupported())
        }
        pub fn incoming(&self) -> std::iter::Empty<io::Result<UnixStream>> {
            match *self {}
        }
        pub fn accept(&self) -> io::Result<(UnixStream, ())> {
            match *self {}
        }
    }
}
