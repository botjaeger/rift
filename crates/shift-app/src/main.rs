mod app;
mod capture;
mod hotkey;
mod input;
#[cfg(target_os = "macos")]
mod menubar;
mod mouse;
mod notify;
mod pane;
mod script;
mod state;
mod views;
mod vim;

use winit::event_loop::EventLoop;

const USAGE: &str = "usage: shift-terminal [--script <file>] [--version] [--help]";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut script = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--script" => {
                let Some(path) = args.next() else {
                    eprintln!("{USAGE}");
                    std::process::exit(2);
                };
                match script::Script::load(std::path::Path::new(&path)) {
                    Ok(s) => script = Some(s),
                    Err(e) => {
                        eprintln!("shift-terminal: --script: {e}");
                        std::process::exit(2);
                    }
                }
            }
            "--version" | "-V" => {
                println!("shift-terminal {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return;
            }
            other => {
                eprintln!("shift-terminal: unknown argument {other:?}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let event_loop = EventLoop::<app::UserEvent>::with_user_event()
        .build()
        .expect("create event loop");
    let proxy = event_loop.create_proxy();

    let mut application = app::App::new(proxy, script);
    event_loop.run_app(&mut application).expect("event loop");
}
