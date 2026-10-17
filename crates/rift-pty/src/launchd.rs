//! Starting Rift's background helpers — the session service and the phone
//! gateway — so they outlive the window without keeping Rift in the Dock.
//!
//! On macOS a process the app starts stays in the app's coalition: after
//! the window quits, the Dock keeps Rift as "Running in Background" for as
//! long as a helper runs. macOS 27 does the same for a launchd job the app
//! bootstraps itself (measured 2026-10-17), but not for an agent registered
//! through `SMAppService`: Rift.app carries one property list per helper in
//! `Contents/Library/LaunchAgents` (`dev.rift.<name>.plist`), registered and
//! then started on demand. Those lists are fixed, so a run with its own data
//! or config directory (`XDG_DATA_HOME`, `XDG_CONFIG_HOME`: scripted and
//! test runs), a run outside the bundle and macOS before 13 bootstrap a
//! one-shot job per data directory instead (no keep-alive, gone from
//! launchd's care when it exits). Elsewhere, or when launchd refuses, the
//! helper is a plain detached child as before.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Start `program args…` detached from this process, its output appended
/// to `log` (or discarded). `name` tells this helper's job apart (`session`,
/// `gateway`); the job is per data directory, so isolated runs keep theirs.
pub fn spawn_helper(
    name: &str,
    program: &Path,
    args: &[&str],
    log: Option<&Path>,
) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(plist) = bundled_agent(name, program) {
        match sm::start_agent(&plist) {
            Ok(()) => return Ok(()),
            Err(e) => log::warn!("{name}: SMAppService refused the agent ({e}); bootstrapping it"),
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(data) = rift_core::shell_integration::data_dir() {
        match spawn_job(&data.join("launchd"), name, program, args, log) {
            Ok(()) => return Ok(()),
            Err(e) => log::warn!("{name}: launchd refused the job ({e}); starting it as a child"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = name;
    spawn_child(program, args, log)
}

/// The pre-launchd way, and the fallback: a child in its own process group.
fn spawn_child(program: &Path, args: &[&str], log: Option<&Path>) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    let (out, err) = match log {
        Some(path) => {
            let f = append(path)?;
            (Stdio::from(f.try_clone()?), Stdio::from(f))
        }
        None => (Stdio::null(), Stdio::null()),
    };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()?;
    // no zombie when it exits while this process lives on
    std::thread::Builder::new()
        .name("helper-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

/// The label of `name`'s agent in the bundle that holds `program`, when
/// this run may use it: `program` sits in `<bundle>/Contents/MacOS`, the
/// bundle carries `Contents/Library/LaunchAgents/dev.rift.<name>.plist`,
/// and no data or config directory of its own is set (the agent's list
/// is fixed and runs with launchd's environment).
#[cfg(target_os = "macos")]
fn bundled_agent(name: &str, program: &Path) -> Option<String> {
    let isolated = ["XDG_DATA_HOME", "XDG_CONFIG_HOME"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
    bundled_agent_in(name, program, isolated)
}

#[cfg(any(target_os = "macos", test))]
fn bundled_agent_in(name: &str, program: &Path, isolated: bool) -> Option<String> {
    if isolated {
        return None;
    }
    let contents = program.parent()?.parent()?;
    if program.parent()?.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    let label = format!("dev.rift.{name}");
    contents
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist"))
        .is_file()
        .then_some(label)
}

/// Whether Rift opens at login (its login item, `SMAppService`); `None`
/// where that cannot be known (macOS before 13, other systems).
pub fn launch_at_login() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        sm::main_app_enabled()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Add or remove Rift's login item. Fails before macOS 13, on other
/// systems, and for a run outside Rift.app.
pub fn set_launch_at_login(on: bool) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        sm::set_main_app(on)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = on;
        Err(io::Error::other("launch at login is macOS only"))
    }
}

/// `SMAppService` (ServiceManagement, macOS 13+), looked up at run time:
/// the app still starts on macOS 11 and 12, where helpers bootstrap and
/// there is no login item.
#[cfg(target_os = "macos")]
mod sm {
    use std::io;

    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSError, NSString};

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    /// `SMAppServiceStatus`
    const ENABLED: isize = 1;
    const REQUIRES_APPROVAL: isize = 2;

    fn class() -> io::Result<&'static AnyClass> {
        AnyClass::get(c"SMAppService").ok_or_else(|| io::Error::other("needs macOS 13"))
    }

    fn agent(plist: &str) -> io::Result<Retained<AnyObject>> {
        let name = NSString::from_str(plist);
        // SAFETY: `+agentServiceWithPlistName:` takes an NSString and
        // returns an autoreleased SMAppService (never nil per the header)
        let svc: Option<Retained<AnyObject>> =
            unsafe { msg_send![class()?, agentServiceWithPlistName: &*name] };
        svc.ok_or_else(|| io::Error::other("no SMAppService for the agent"))
    }

    fn main_app() -> io::Result<Retained<AnyObject>> {
        // SAFETY: `+mainAppService` takes nothing and returns the service
        let svc: Option<Retained<AnyObject>> = unsafe { msg_send![class()?, mainAppService] };
        svc.ok_or_else(|| io::Error::other("no SMAppService for the app"))
    }

    fn status(svc: &AnyObject) -> isize {
        // SAFETY: `-status` returns SMAppServiceStatus (NSInteger)
        unsafe { msg_send![svc, status] }
    }

    fn register(svc: &AnyObject) -> io::Result<()> {
        // SAFETY: `-registerAndReturnError:` returns BOOL with NSError**
        let r: Result<(), Retained<NSError>> = unsafe { msg_send![svc, registerAndReturnError: _] };
        r.map_err(|e| io::Error::other(e.localizedDescription().to_string()))
    }

    fn unregister(svc: &AnyObject) -> io::Result<()> {
        // SAFETY: `-unregisterAndReturnError:` returns BOOL with NSError**
        let r: Result<(), Retained<NSError>> =
            unsafe { msg_send![svc, unregisterAndReturnError: _] };
        r.map_err(|e| io::Error::other(e.localizedDescription().to_string()))
    }

    /// Register agent `label` (once; it stays registered across launches
    /// and updates) and start it unless it runs (`kickstart` without `-k`).
    pub fn start_agent(label: &str) -> io::Result<()> {
        let svc = agent(&format!("{label}.plist"))?;
        if status(&svc) != ENABLED {
            register(&svc)?;
            if status(&svc) != ENABLED {
                // turned off in System Settings › Login Items
                return Err(io::Error::other("not allowed in Login Items"));
            }
        }
        let target = format!("gui/{}/{label}", unsafe { libc::getuid() });
        super::launchctl(&["kickstart", &target])
    }

    pub fn main_app_enabled() -> Option<bool> {
        let svc = main_app().ok()?;
        Some(matches!(status(&svc), ENABLED | REQUIRES_APPROVAL))
    }

    pub fn set_main_app(on: bool) -> io::Result<()> {
        let svc = main_app()?;
        let st = status(&svc);
        match (on, st) {
            (true, ENABLED) => Ok(()),
            (true, _) => register(&svc),
            (false, ENABLED | REQUIRES_APPROVAL) => unregister(&svc),
            (false, _) => Ok(()),
        }
    }
}

fn append(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(target_os = "macos")]
fn spawn_job(
    dir: &Path,
    name: &str,
    program: &Path,
    args: &[&str],
    log: Option<&Path>,
) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let label = job_label(name, dir);
    let target = format!("gui/{}/{label}", unsafe { libc::getuid() });
    if job_running(&target) {
        // another window started it a moment ago; the caller waits for it
        return Ok(());
    }
    let env: Vec<(String, String)> = std::env::vars().collect();
    let plist = job_plist(&label, program, args, &env, log);
    let path = dir.join(format!("{label}.plist"));
    std::fs::write(&path, plist)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    // a job that ran and ended stays loaded: replace it (it is not running)
    let _ = launchctl(&["bootout", &target]);
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    launchctl(&["bootstrap", &domain, &path.to_string_lossy()])
}

#[cfg(target_os = "macos")]
fn job_running(target: &str) -> bool {
    Command::new("/bin/launchctl")
        .args(["print", target])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).contains("state = running")
        })
}

#[cfg(target_os = "macos")]
fn launchctl(args: &[&str]) -> io::Result<()> {
    let out = Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "launchctl {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// `dev.rift.<name>.<hash of the data dir>`: one job per helper and data
/// directory (a scripted run's scratch directory gets its own).
#[cfg(any(target_os = "macos", test))]
fn job_label(name: &str, dir: &Path) -> String {
    // FNV-1a: stable across builds, unlike the std hasher
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in dir.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("dev.rift.{name}.{:08x}", h as u32)
}

/// The job's property list: the program and this process's environment
/// (the helper sees what a child would have), run once at load, never kept
/// alive, scheduled like an app (shells run under it).
#[cfg(any(target_os = "macos", test))]
fn job_plist(
    label: &str,
    program: &Path,
    args: &[&str],
    env: &[(String, String)],
    log: Option<&Path>,
) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    let key = |s: &mut String, k: &str| s.push_str(&format!("  <key>{}</key>\n", xml(k)));
    let string = |s: &mut String, v: &str| s.push_str(&format!("  <string>{}</string>\n", xml(v)));
    key(&mut s, "Label");
    string(&mut s, label);
    key(&mut s, "ProgramArguments");
    s.push_str("  <array>\n");
    string(&mut s, &program.to_string_lossy());
    for a in args {
        string(&mut s, a);
    }
    s.push_str("  </array>\n");
    key(&mut s, "EnvironmentVariables");
    s.push_str("  <dict>\n");
    for (k, v) in env {
        key(&mut s, k);
        string(&mut s, v);
    }
    s.push_str("  </dict>\n");
    if let Some(home) = env.iter().find(|(k, _)| k == "HOME") {
        key(&mut s, "WorkingDirectory");
        string(&mut s, &home.1);
    }
    let out = log.map_or_else(|| std::path::PathBuf::from("/dev/null"), Path::to_path_buf);
    key(&mut s, "StandardOutPath");
    string(&mut s, &out.to_string_lossy());
    key(&mut s, "StandardErrorPath");
    string(&mut s, &out.to_string_lossy());
    key(&mut s, "RunAtLoad");
    s.push_str("  <true/>\n");
    key(&mut s, "KeepAlive");
    s.push_str("  <false/>\n");
    // the shells it starts are its own business when it exits
    key(&mut s, "AbandonProcessGroup");
    s.push_str("  <true/>\n");
    key(&mut s, "ProcessType");
    string(&mut s, "Interactive");
    s.push_str("</dict>\n</plist>\n");
    s
}

/// Escape text for a property list (control characters other than tab and
/// newline are not allowed in XML 1.0 and are dropped).
#[cfg(any(target_os = "macos", test))]
fn xml(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_per_helper_and_data_dir() {
        let a = job_label("session", Path::new("/Users/a/.local/share/rift/launchd"));
        assert!(a.starts_with("dev.rift.session."), "{a}");
        assert_eq!(
            a,
            job_label("session", Path::new("/Users/a/.local/share/rift/launchd"))
        );
        assert_ne!(
            a,
            job_label("session", Path::new("/tmp/scratch/rift/launchd"))
        );
        assert_ne!(
            a,
            job_label("gateway", Path::new("/Users/a/.local/share/rift/launchd"))
        );
    }

    #[test]
    fn bundled_agents_only_for_a_shared_run_inside_the_bundle() {
        let scratch = std::env::temp_dir().join(format!("rift-agents-{}", std::process::id()));
        let contents = scratch.join("Rift.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Library/LaunchAgents")).unwrap();
        std::fs::write(
            contents.join("Library/LaunchAgents/dev.rift.session.plist"),
            "",
        )
        .unwrap();
        let exe = contents.join("MacOS/rift-terminal");
        assert_eq!(
            bundled_agent_in("session", &exe, false).as_deref(),
            Some("dev.rift.session")
        );
        // no list for it, its own data dir, or not in a bundle: bootstrap
        assert_eq!(bundled_agent_in("gateway", &exe, false), None);
        assert_eq!(bundled_agent_in("session", &exe, true), None);
        let loose = scratch.join("target/release/rift-terminal");
        assert_eq!(bundled_agent_in("session", &loose, false), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_job_runs_once_with_this_environment() {
        let env = vec![
            ("HOME".to_string(), "/Users/a".to_string()),
            ("ODD".to_string(), "a<b & c>\u{1}".to_string()),
        ];
        let p = job_plist(
            "dev.rift.session.1",
            Path::new("/Applications/Rift.app/Contents/MacOS/rift-terminal"),
            &["--session-daemon"],
            &env,
            None,
        );
        assert!(p.contains("<string>/Applications/Rift.app/Contents/MacOS/rift-terminal</string>\n  <string>--session-daemon</string>"), "{p}");
        assert!(p.contains("<key>KeepAlive</key>\n  <false/>"), "{p}");
        assert!(p.contains("<key>RunAtLoad</key>\n  <true/>"), "{p}");
        assert!(
            p.contains("<key>ODD</key>\n  <string>a&lt;b &amp; c&gt;</string>"),
            "{p}"
        );
        assert!(
            p.contains("<key>WorkingDirectory</key>\n  <string>/Users/a</string>"),
            "{p}"
        );
        assert!(
            p.contains("<key>StandardOutPath</key>\n  <string>/dev/null</string>"),
            "{p}"
        );
    }

    /// A real job: started by launchd, not as a child of this process (the
    /// Dock then counts it as Rift's). Keep-alive is `job_plist`'s test.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_helper_runs_under_launchd() {
        let scratch = std::env::temp_dir().join(format!("rift-launchd-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let marker = scratch.join("ran");
        let script = format!("echo $PPID >> {}; sleep 1", marker.display());
        let dir = scratch.join("launchd");
        spawn_job(&dir, "test", Path::new("/bin/sh"), &["-c", &script], None).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let ran = std::fs::read_to_string(&marker).unwrap_or_default();
        let target = format!(
            "gui/{}/{}",
            unsafe { libc::getuid() },
            job_label("test", &dir)
        );
        let _ = launchctl(&["bootout", &target]);
        let _ = std::fs::remove_dir_all(&scratch);
        // its parent is launchd (pid 1), not this test
        assert_eq!(ran.lines().collect::<Vec<_>>(), ["1"], "{ran:?}");
    }
}
