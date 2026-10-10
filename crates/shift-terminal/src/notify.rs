//! Desktop notifications and the audible bell, via the platform's own
//! tools (no extra dependencies): `osascript` on macOS, `notify-send` /
//! PulseAudio on Linux. Every call is fire-and-forget.

use std::process::{Command, Stdio};

fn spawn(cmd: &str, args: &[&str]) {
    let r = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match r {
        // reap it off the main thread, or every sound leaves a zombie
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::debug!("notify: {cmd}: {e}"),
    }
}

/// AppleScript string literal: escape backslashes and double quotes.
fn applescript_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Desktop notification (title + body).
pub fn send(title: &str, body: &str) {
    if std::env::var_os("SHIFT_NO_NOTIFY").is_some() {
        log::info!("notify (suppressed): {title} — {body}");
        return;
    }
    if cfg!(target_os = "macos") {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            applescript_str(body),
            applescript_str(title)
        );
        spawn("osascript", &["-e", &script]);
    } else {
        spawn("notify-send", &["--app-name=Shift", title, body]);
    }
}

/// Play an audio file (notification sounds, Settings preview).
pub fn play(path: &std::path::Path) {
    let p = path.to_string_lossy();
    if std::env::var_os("SHIFT_NO_NOTIFY").is_some() {
        log::info!("sound (suppressed): {p}");
        return;
    }
    log::info!("sound: {p}");
    if cfg!(target_os = "macos") {
        spawn("afplay", &[&p]);
    } else if which("paplay") {
        spawn("paplay", &[&p]);
    } else {
        spawn("canberra-gtk-play", &["-f", &p]);
    }
}

fn which(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(cmd).is_file()))
}

/// Audible bell.
pub fn beep() {
    if std::env::var_os("SHIFT_NO_NOTIFY").is_some() {
        return;
    }
    if cfg!(target_os = "macos") {
        spawn("afplay", &["/System/Library/Sounds/Tink.aiff"]);
    } else if std::path::Path::new("/usr/share/sounds/freedesktop/stereo/bell.oga").exists() {
        spawn("paplay", &["/usr/share/sounds/freedesktop/stereo/bell.oga"]);
    } else {
        spawn("canberra-gtk-play", &["-i", "bell"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_escaping() {
        assert_eq!(applescript_str(r#"say "hi" \ bye"#), r#"say \"hi\" \\ bye"#);
    }
}
