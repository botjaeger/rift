mod a11y;
mod agent_status;
mod app;
mod background;
mod block_export;
mod capture;
mod composer;
mod control;
mod dropdown;
mod file_link;
mod hotkey;
mod icon;
mod input;
mod keyboard;
#[cfg(target_os = "macos")]
mod links;
#[cfg(target_os = "macos")]
mod menubar;
mod mouse;
mod notify;
mod pane;
mod progress;
mod quit;
mod remote;
mod results;
mod script;
mod secret_mask;
mod smart_select;
mod stall;
mod state;
#[cfg(target_os = "macos")]
mod status_item;
mod subshell;
mod tab_status;
#[cfg(target_os = "macos")]
mod traffic_lights;
#[cfg(target_os = "macos")]
mod translucency;
mod tunnel;
mod updater;
mod views;
mod vim;

use winit::event_loop::EventLoop;

const USAGE: &str = "usage: rift-terminal [--new-window] [PATH…]   a folder (or a file's folder) in a new tab\n       rift-terminal [rift://LINK] [--script <file> [--headless]] [--version] [--help]\n       rift-terminal ctl <command>   (see `rift-terminal ctl help`)\n       rift-terminal remote [--listen ADDR] [--managed] | pair [--view] | devices | revoke ID | history [--limit N] | permission ID operate|view\n                                     gateway for the mobile companion (see `rift-terminal remote help`)\n       rift-terminal relay [--listen ADDR] [--trust-forwarded]\n                                     relay for phones outside the local network (see `rift-terminal relay help`)";

/// The `rift://` link that opens `path` (relative to this process's
/// directory) in a new tab or window.
fn open_path_link(path: &std::path::Path, new_window: bool) -> Result<String, String> {
    let abs = std::path::absolute(path).map_err(|e| format!("{}: {e}", path.display()))?;
    rift_core::uri::open_path_link(&abs, new_window)
}

fn main() {
    // restart variables are read once and not passed on to shells
    updater::take_env();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut script = None;
    let mut headless = false;
    let mut links: Vec<String> = Vec::new();
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    let mut new_window = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            // debugging/testing: the scripted window is never shown and
            // never takes focus, so the user's own typing cannot reach it
            "--headless" => {
                headless = true;
                // the gateway this run starts shows no menu bar icon either
                // SAFETY: still single-threaded (no window, no worker yet)
                unsafe { std::env::set_var(remote::HEADLESS_ENV, "1") };
            }
            "--script" => {
                let Some(path) = args.next() else {
                    eprintln!("{USAGE}");
                    std::process::exit(2);
                };
                match script::Script::load(std::path::Path::new(&path)) {
                    Ok(s) => {
                        script = Some(s);
                        // tests never register at the public relay; the
                        // gateway this run starts inherits it
                        // SAFETY: still single-threaded (no window, no
                        // worker started yet)
                        unsafe { std::env::set_var(rift_core::config::NO_DEFAULT_RELAY_ENV, "1") };
                    }
                    Err(e) => {
                        eprintln!("rift-terminal: --script: {e}");
                        std::process::exit(2);
                    }
                }
            }
            // the per-user session service (started on demand by windows
            // with `durable_sessions = true`)
            "--session-daemon" => {
                if let Err(e) = rift_pty::daemon::run() {
                    eprintln!("rift-terminal: session service: {e}");
                    std::process::exit(1);
                }
                return;
            }
            "ctl" => {
                let rest: Vec<String> = args.collect();
                std::process::exit(control::cli(&rest));
            }
            "remote" => {
                let rest: Vec<String> = args.collect();
                std::process::exit(remote::cli(&rest));
            }
            // the relay (same code as the slim `rift-relay` binary)
            "relay" => {
                let rest: Vec<String> = args.collect();
                std::process::exit(rift_remote::relay::cli(&rest));
            }
            "--version" | "-V" => {
                println!("rift-terminal {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return;
            }
            // a `rift://` link (Linux desktops pass it as `%U`): the running
            // Rift opens it, else this process does once it is up
            l if l.starts_with("rift://") => links.push(l.to_string()),
            // LaunchServices' process serial number (older macOS, some
            // launchers): not ours to parse
            psn if psn.starts_with("-psn_") => {}
            // the folders below open in a new window rather than a tab
            "--new-window" => new_window = true,
            // a desktop launcher's `%U` passes local files as file:// URLs
            url if url.starts_with("file://") => match rift_core::uri::file_url_path(url) {
                Some(p) => paths.push(p),
                None => {
                    eprintln!("rift-terminal: not a local file: {url}");
                    std::process::exit(2);
                }
            },
            // a folder or file to open (an editor's "external terminal",
            // a script), relative to the caller's directory
            path if !path.starts_with('-') => paths.push(path.into()),
            other => {
                eprintln!("rift-terminal: unknown argument {other:?}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    if headless && script.is_none() {
        eprintln!("rift-terminal: --headless needs --script\n{USAGE}");
        std::process::exit(2);
    }
    if new_window && paths.is_empty() {
        links.push(format!("{}://action/new_window", rift_core::uri::SCHEME));
    }
    for path in &paths {
        match open_path_link(path, new_window) {
            Ok(l) => links.push(l),
            Err(e) => {
                eprintln!("rift-terminal: {e}");
                std::process::exit(2);
            }
        }
    }
    // the running Rift opens them; what it did not take, this process
    // opens once it is up
    if script.is_none() && !links.is_empty() {
        links.retain(|l| !control::forward_link(l));
        if links.is_empty() {
            return;
        }
    }

    // the window's process only (not `ctl`, `remote` or the services): its
    // main thread blocked longer than a moment is written down
    stall::start();
    stall::busy(stall::Phase::Running);
    let mut builder = EventLoop::<app::UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    if headless {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        // no Dock icon or menu bar, and launching does not steal focus
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false);
    }
    let event_loop = builder.build().expect("create event loop");
    let proxy = event_loop.create_proxy();
    // LaunchServices sends links only to a bundle that declares the scheme
    // (`Rift.app`), never to a bare `cargo run` or test binary
    #[cfg(target_os = "macos")]
    links::install(proxy.clone());

    let mut application = app::App::new(proxy, script, headless);
    // a fresh start opens its first tab in the first link's folder
    application.start_links = links;
    event_loop.run_app(&mut application).expect("event loop");
    quit::LOOP_RETURNED.store(true, std::sync::atomic::Ordering::SeqCst);
    // the last commands' history entries land before the process ends
    application.flush_command_history();
    // ended on purpose: the next launch does not take it for a crash
    rift_core::session_state::mark_clean_exit();
    // a process that does not end now is a stall too (the Dock keeps it)
    stall::busy(stall::Phase::Quitting);
    // the warm completion shells (and what their functions started) end
    // with the app: they never outlive it, a hung function included
    rift_core::shell_complete::stop_all();
}
