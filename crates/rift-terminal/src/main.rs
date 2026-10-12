mod a11y;
mod app;
mod capture;
mod control;
mod hotkey;
mod icon;
mod input;
mod keyboard;
#[cfg(target_os = "macos")]
mod menubar;
mod mouse;
mod notify;
mod pane;
mod script;
mod state;
#[cfg(target_os = "macos")]
mod traffic_lights;
#[cfg(target_os = "macos")]
mod translucency;
mod updater;
mod views;
mod vim;

use winit::event_loop::EventLoop;

const USAGE: &str = "usage: rift-terminal [--script <file> [--headless]] [--version] [--help]\n       rift-terminal ctl <command>   (see `rift-terminal ctl help`)";

fn main() {
    // restart variables are read once and not passed on to shells
    updater::take_env();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut script = None;
    let mut headless = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            // debugging/testing: the scripted window is never shown and
            // never takes focus, so the user's own typing cannot reach it
            "--headless" => headless = true,
            "--script" => {
                let Some(path) = args.next() else {
                    eprintln!("{USAGE}");
                    std::process::exit(2);
                };
                match script::Script::load(std::path::Path::new(&path)) {
                    Ok(s) => script = Some(s),
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
            "--version" | "-V" => {
                println!("rift-terminal {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return;
            }
            // LaunchServices' process serial number (older macOS, some
            // launchers): not ours to parse
            psn if psn.starts_with("-psn_") => {}
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

    let mut application = app::App::new(proxy, script, headless);
    event_loop.run_app(&mut application).expect("event loop");
}
