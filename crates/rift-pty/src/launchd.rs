//! Starting Rift's background helpers — the session service and the phone
//! gateway — so they outlive the window without keeping Rift in the Dock.
//!
//! On macOS a process the app starts stays in the app's coalition: after
//! the window quits, the Dock keeps Rift as "Running in Background" for as
//! long as a helper runs (macOS 27 does the same for a launchd job the app
//! bootstraps itself, measured 2026-10-17). A helper that is a process of
//! Rift.app itself has two more problems: macOS counts it as the running
//! Rift and hands it every launch of the app (the Dock, Spotlight, `open -a
//! Rift`: a Dock icon and no window), and an agent registered through
//! `SMAppService` is pinned to one build, so launchd kills the helper of any
//! other build ("rift-terminal quit unexpectedly", 2026-10-19). So Rift.app
//! carries `Rift Helper.app` (`Contents/Library/LoginItems`, its own bundle
//! identifier, no Dock icon) and LaunchServices starts each helper as that
//! app (`open -n -g -j`): outside Rift's coalition, not the running Rift,
//! nothing registered and nothing pinned. A run with its own data or config
//! directory (`XDG_DATA_HOME`, `XDG_CONFIG_HOME`: scripted and test runs),
//! a run outside the bundle and a failed `open` bootstrap a one-shot job per
//! data directory instead (no keep-alive, gone from launchd's care when it
//! exits). Elsewhere, or when launchd refuses, the helper is a plain
//! detached child as before.

use std::io;
use std::path::Path;
#[cfg(any(target_os = "macos", test))]
use std::path::PathBuf;
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
    if let Some(app) = bundled_helper(program) {
        match open_helper(&app, name, args) {
            Ok(()) => return Ok(()),
            Err(e) => log::warn!("{name}: could not open the helper app ({e}); bootstrapping it"),
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

/// The helper app inside the bundle `program` belongs to, when this run may
/// use it: `program` sits in `<bundle>/Contents/MacOS` of Rift.app (or of
/// the helper app itself, whose own helpers are its siblings), the bundle
/// carries `Contents/Library/LoginItems/Rift Helper.app`, and no data or
/// config directory of its own is set (a helper app is launched by
/// LaunchServices, with no environment of ours).
#[cfg(target_os = "macos")]
fn bundled_helper(program: &Path) -> Option<PathBuf> {
    let isolated = ["XDG_DATA_HOME", "XDG_CONFIG_HOME"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
    bundled_helper_in(program, isolated)
}

/// The helpers start as `Rift Helper.app` (so macOS holds that app, not
/// Rift, responsible for the shells they run): false for an isolated run, a
/// run outside the bundle, a bundle without the helper and off macOS.
pub fn runs_as_helper_app() -> bool {
    helper_app().is_some()
}

/// `Rift Helper.app` of this run's bundle, when the helpers start as it
/// (Settings › Permissions shows it in Finder, to drag into a privacy list).
pub fn helper_app() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    if let Ok(exe) = std::env::current_exe() {
        return bundled_helper(&exe);
    }
    None
}

#[cfg(any(target_os = "macos", test))]
fn bundled_helper_in(program: &Path, isolated: bool) -> Option<PathBuf> {
    if isolated {
        return None;
    }
    let contents = program.parent()?.parent()?;
    if program.parent()?.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    // a helper's own helpers: the app it runs from
    if bundle.file_name()? == HELPER_APP {
        return Some(bundle.to_path_buf());
    }
    let app = contents.join("Library/LoginItems").join(HELPER_APP);
    app.is_dir().then_some(app)
}

/// The helper app's folder name in `Contents/Library/LoginItems`.
#[cfg(any(target_os = "macos", test))]
const HELPER_APP: &str = "Rift Helper.app";

/// How long `open` may take to hand the helper to LaunchServices.
#[cfg(target_os = "macos")]
const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Start `args` in a new instance of the helper app (`-n`: another helper of
/// the same app may run; `-g -j`: no focus, hidden).
#[cfg(target_os = "macos")]
fn open_helper(app: &Path, name: &str, args: &[&str]) -> io::Result<()> {
    let mut cmd = Command::new("/usr/bin/open");
    cmd.args(open_args(app, name, args));
    let out = output_within(cmd, OPEN_TIMEOUT)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "open: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// The arguments of `open` that start `args` in a new instance of the helper
/// app at `app`. The gateway writes `<data dir>/remote/gateway.log` itself
/// (`RIFT_GATEWAY_LOG`): no path fits a fixed list.
#[cfg(any(target_os = "macos", test))]
fn open_args(app: &Path, name: &str, args: &[&str]) -> Vec<std::ffi::OsString> {
    let mut out: Vec<std::ffi::OsString> = vec!["-n".into(), "-g".into(), "-j".into(), app.into()];
    if name == "gateway" {
        out.extend(["--env".into(), "RIFT_GATEWAY_LOG=1".into()]);
    }
    out.push("--args".into());
    out.extend(args.iter().map(std::ffi::OsString::from));
    out
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
/// the login item. The app still starts on macOS 11 and 12, where there is
/// none.
#[cfg(target_os = "macos")]
mod sm {
    use std::io;

    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSError;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    /// `SMAppServiceStatus`
    const ENABLED: isize = 1;
    const REQUIRES_APPROVAL: isize = 2;

    fn class() -> io::Result<&'static AnyClass> {
        AnyClass::get(c"SMAppService").ok_or_else(|| io::Error::other("needs macOS 13"))
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
    match job_state(&target) {
        // another window started it a moment ago; the caller waits for it
        JobState::Loaded(s) if s.contains("state = running") => return Ok(()),
        // launchctl could not tell: replacing the job could end a running
        // service
        JobState::Unknown(e) => return Err(e),
        _ => {}
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

/// What launchd says about a job.
#[cfg(target_os = "macos")]
enum JobState {
    /// `launchctl print`'s description of it.
    Loaded(String),
    /// launchd has no such job (`print` exits 113).
    NotLoaded,
    /// launchctl failed or did not answer: nothing is known.
    Unknown(io::Error),
}

#[cfg(target_os = "macos")]
fn job_state(target: &str) -> JobState {
    match launchctl_output(&["print", target]) {
        Ok(out) if out.status.success() => {
            JobState::Loaded(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        Ok(out) if out.status.code() == Some(113) => JobState::NotLoaded,
        Ok(out) => JobState::Unknown(io::Error::other(format!(
            "launchctl print: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        Err(e) => JobState::Unknown(e),
    }
}

/// How long one `launchctl` call may take: it waits on launchd, which can
/// itself wait (a job it cannot spawn left `kickstart` hanging for minutes,
/// 2026-10-18).
#[cfg(target_os = "macos")]
const LAUNCHCTL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(target_os = "macos")]
fn launchctl_output(args: &[&str]) -> io::Result<std::process::Output> {
    let mut cmd = Command::new("/bin/launchctl");
    cmd.args(args);
    output_within(cmd, LAUNCHCTL_TIMEOUT).map_err(|e| match e.kind() {
        io::ErrorKind::TimedOut => io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "launchctl {} did not answer",
                args.first().copied().unwrap_or("")
            ),
        ),
        _ => e,
    })
}

/// Run `cmd` and collect its output, giving up (and killing it) after
/// `limit`. Both pipes are read while it runs: `launchctl print` lists the
/// job's environment, which can outgrow a pipe and stall it until read.
#[cfg(any(target_os = "macos", test))]
fn output_within(mut cmd: Command, limit: std::time::Duration) -> io::Result<std::process::Output> {
    use std::io::Read;
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let until = std::time::Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if std::time::Instant::now() >= until {
            // killing it closes the pipes, which ends the readers
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::ErrorKind::TimedOut.into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    Ok(std::process::Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

#[cfg(target_os = "macos")]
fn launchctl(args: &[&str]) -> io::Result<()> {
    let out = launchctl_output(args)?;
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
    fn output_within_reads_large_output_and_gives_up_on_a_hang() {
        use std::time::{Duration, Instant};
        // more than a pipe holds, on both streams: read while it runs
        let mut big = Command::new("/bin/sh");
        big.args([
            "-c",
            "head -c 300000 /dev/zero; head -c 200000 /dev/zero >&2",
        ]);
        // a full pipe would stall it until the limit, which is an error: no
        // wall-clock bound here (3.6 s on a loaded machine, 2026-11-07)
        let out = output_within(big, Duration::from_secs(5)).unwrap();
        assert_eq!((out.stdout.len(), out.stderr.len()), (300_000, 200_000));
        // a command that does not end is killed at the limit
        let mut hang = Command::new("/bin/sleep");
        hang.arg("30");
        let t = Instant::now();
        let err = output_within(hang, Duration::from_millis(300)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

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
    fn the_helper_app_only_for_a_shared_run_inside_the_bundle() {
        let scratch = std::env::temp_dir().join(format!("rift-helper-app-{}", std::process::id()));
        let contents = scratch.join("Rift.app/Contents");
        let helper = contents.join("Library/LoginItems/Rift Helper.app");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(helper.join("Contents/MacOS")).unwrap();
        let exe = contents.join("MacOS/rift-terminal");
        assert_eq!(bundled_helper_in(&exe, false), Some(helper.clone()));
        // a helper's own helpers run from the same app
        let from_helper = helper.join("Contents/MacOS/rift-terminal");
        assert_eq!(bundled_helper_in(&from_helper, false), Some(helper));
        // its own data dir, or not in a bundle: bootstrap
        assert_eq!(bundled_helper_in(&exe, true), None);
        let loose = scratch.join("target/release/rift-terminal");
        assert_eq!(bundled_helper_in(&loose, false), None);
        // a bundle without the helper app (an older Rift.app)
        let old = scratch.join("Old.app/Contents/MacOS/rift-terminal");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        assert_eq!(bundled_helper_in(&old, false), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_helper_is_a_new_hidden_instance_of_its_app() {
        let app = Path::new("/Applications/Rift.app/Contents/Library/LoginItems/Rift Helper.app");
        let args = |name, a: &[&str]| -> Vec<String> {
            open_args(app, name, a)
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(
            args("session", &["--session-daemon"]),
            [
                "-n",
                "-g",
                "-j",
                "/Applications/Rift.app/Contents/Library/LoginItems/Rift Helper.app",
                "--args",
                "--session-daemon"
            ]
        );
        // the gateway keeps its own log
        assert_eq!(
            args("gateway", &["remote", "--managed"])[4..],
            [
                "--env",
                "RIFT_GATEWAY_LOG=1",
                "--args",
                "remote",
                "--managed"
            ]
        );
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
