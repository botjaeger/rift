//! `rift-relay`: the relay alone (std + tungstenite, no GUI), for servers
//! and `packaging/relay.Dockerfile`. `rift-terminal relay` runs the same
//! code ([`rift_remote::relay::cli`]).

/// One line per record on stderr at `RUST_LOG`'s level (`info` when unset;
/// the relay never logs frame contents).
struct Stderr(log::LevelFilter);

impl log::Log for Stderr {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= self.0
    }

    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("{} {}", r.level(), r.args());
        }
    }

    fn flush(&self) {}
}

extern "C" fn exit_now(_: libc::c_int) {
    // SAFETY: _exit is async-signal-safe
    unsafe { libc::_exit(0) }
}

fn main() {
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|l| l.parse().ok())
        .unwrap_or(log::LevelFilter::Info);
    let logger: &'static Stderr = Box::leak(Box::new(Stderr(level)));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
    // as PID 1 in a container SIGTERM is ignored by default: stop at once
    // (no state to save; peers reconnect)
    // SAFETY: installs a handler that only calls _exit
    unsafe {
        libc::signal(libc::SIGTERM, exit_now as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, exit_now as *const () as libc::sighandler_t);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(rift_remote::relay::cli(&args));
}
