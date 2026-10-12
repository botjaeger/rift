//! The Docker Compose project around a directory, for the prompt's Docker
//! chip: the nearest compose file up the tree, and its containers from
//! `docker compose ps` on a background thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

/// Compose file names, in Compose's own lookup order.
const FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

/// The nearest compose file at or above `dir`, stopping at the home
/// directory (a stray `~/docker-compose.yml` is not every folder's project).
pub fn find_compose(dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    for d in dir.ancestors() {
        if let Some(f) = FILES.iter().map(|f| d.join(f)).find(|p| p.is_file()) {
            return Some(f);
        }
        if Some(d) == home {
            break;
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub service: String,
    pub name: String,
    /// `running`, `exited`, `paused`…
    pub state: String,
    /// `Up 2 hours (healthy)`, `Exited (0) 3 minutes ago`
    pub status: String,
    pub image: String,
    /// Published ports: `8080→80` (TCP), `5353→53/udp`.
    pub ports: Vec<String>,
    /// The health check's verdict, when the service has one.
    pub health: Option<Health>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Unhealthy,
    Starting,
}

impl Health {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "healthy" => Some(Health::Healthy),
            "unhealthy" => Some(Health::Unhealthy),
            "starting" | "health: starting" => Some(Health::Starting),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Health::Healthy => "healthy",
            Health::Unhealthy => "unhealthy",
            Health::Starting => "starting",
        }
    }
}

impl Container {
    pub fn running(&self) -> bool {
        self.state == "running"
    }

    /// How long it has been up (`5 hours`, `about a minute`) for a running
    /// container; the whole status otherwise (`Exited (0) 3 minutes ago`).
    pub fn uptime(&self) -> String {
        // the health verdict has its own column
        let s = match self.status.rfind(" (") {
            Some(i) if self.status.ends_with(')') && self.status.starts_with("Up ") => {
                &self.status[..i]
            }
            _ => &self.status,
        };
        match s.strip_prefix("Up ") {
            Some(rest) if self.running() => {
                let mut c = rest.chars();
                c.next()
                    .map(|f| f.to_lowercase().chain(c).collect())
                    .unwrap_or_default()
            }
            _ => s.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeInfo {
    /// The compose file's directory name (Compose's default project name).
    pub project: String,
    pub dir: PathBuf,
    pub containers: Vec<Container>,
    /// Why there are no containers to show: no `docker`, daemon down.
    pub error: Option<String>,
}

impl ComposeInfo {
    pub fn running(&self) -> usize {
        self.containers.iter().filter(|c| c.running()).count()
    }
}

/// `docker compose ps --format json`: one object per line (Compose v2.21+)
/// or one array (older).
pub fn parse_ps(out: &str) -> Vec<Container> {
    let values: Vec<serde_json::Value> = match serde_json::from_str(out.trim()) {
        Ok(serde_json::Value::Array(a)) => a,
        Ok(v @ serde_json::Value::Object(_)) => vec![v],
        _ => out
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
    };
    let s = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let mut list: Vec<Container> = values
        .iter()
        .map(|v| {
            let mut ports: Vec<String> = v
                .get("Publishers")
                .and_then(|p| p.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|p| {
                            let published = p.get("PublishedPort")?.as_u64()?;
                            let target = p.get("TargetPort")?.as_u64()?;
                            (published != 0).then(|| {
                                match p.get("Protocol").and_then(|x| x.as_str()) {
                                    None | Some("tcp") => format!("{published}→{target}"),
                                    Some(proto) => format!("{published}→{target}/{proto}"),
                                }
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            // IPv4 and IPv6 publish the same mapping twice
            ports.sort_by_key(|p| {
                p.split('→')
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                    .unwrap_or(u32::MAX)
            });
            ports.dedup();
            let status = s(v, "Status");
            // `Health` (Compose v2), else the status's `(healthy)`
            let health = Health::parse(&s(v, "Health")).or_else(|| {
                let inner = status.rsplit_once(" (")?.1.strip_suffix(')')?;
                Health::parse(inner)
            });
            Container {
                service: s(v, "Service"),
                name: s(v, "Name"),
                state: s(v, "State"),
                status,
                image: s(v, "Image"),
                ports,
                health,
            }
        })
        .collect();
    list.sort_by(|a, b| {
        b.running()
            .cmp(&a.running())
            .then(a.service.cmp(&b.service))
    });
    list
}

const TIMEOUT: Duration = Duration::from_secs(10);

/// `docker` on PATH, else the usual install places (an app started from
/// the Finder has a short PATH).
fn find_docker() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([
            "/usr/local/bin".into(),
            "/opt/homebrew/bin".into(),
            "/Applications/Docker.app/Contents/Resources/bin".into(),
        ])
        .map(|d| d.join("docker"))
        .find(|p| p.is_file())
}

fn query(file: &Path) -> Option<ComposeInfo> {
    let dir = file.parent()?.to_path_buf();
    let project = dir.file_name()?.to_string_lossy().into_owned();
    let info = |containers, error: Option<&str>| ComposeInfo {
        project: project.clone(),
        dir: dir.clone(),
        containers,
        error: error.map(str::to_string),
    };
    let Some(docker) = find_docker() else {
        return Some(info(Vec::new(), Some("docker is not installed")));
    };
    let mut cmd = std::process::Command::new(&docker);
    // the CLI's helpers (credential, compose plugin) live beside it
    if let Some(bin) = docker.parent() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined = std::env::join_paths(
            std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&path)),
        );
        if let Ok(p) = joined {
            cmd.env("PATH", p);
        }
    }
    let child = cmd
        .args(["compose", "-f"])
        .arg(file)
        .args(["ps", "--all", "--format", "json"])
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return Some(info(Vec::new(), Some("docker could not run")));
    };
    let start = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => break false,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    if !ok {
        return Some(info(Vec::new(), Some("Docker is not running")));
    }
    let mut out = String::new();
    use std::io::Read as _;
    child
        .stdout
        .take()?
        .take(1 << 20)
        .read_to_string(&mut out)
        .ok()?;
    Some(info(parse_ps(&out), None))
}

pub type DockerCallback = Arc<dyn Fn(u64, Option<ComposeInfo>) + Send + Sync>;

/// Background `docker compose ps` worker; the newest request per pane wins.
pub struct DockerWorker {
    tx: Sender<(u64, PathBuf)>,
}

impl DockerWorker {
    pub fn new(callback: DockerCallback) -> Self {
        let (tx, rx) = channel::<(u64, PathBuf)>();
        std::thread::Builder::new()
            .name("docker-compose".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    let mut latest = vec![first];
                    while let Ok(next) = rx.try_recv() {
                        latest.retain(|(p, _)| *p != next.0);
                        latest.push(next);
                    }
                    for (pane, cwd) in latest {
                        let home = dirs::home_dir();
                        let info = find_compose(&cwd, home.as_deref()).and_then(|f| query(&f));
                        callback(pane, info);
                    }
                }
            })
            .expect("spawn docker worker");
        DockerWorker { tx }
    }

    /// Look up the compose project around `cwd`.
    pub fn request(&self, pane: u64, cwd: PathBuf) {
        let _ = self.tx.send((pane, cwd));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_nearest_compose_file_below_home() {
        let d = std::env::temp_dir().join(format!("rift-compose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let deep = d.join("home/app/src/x");
        std::fs::create_dir_all(&deep).unwrap();
        let home = d.join("home");
        std::fs::write(home.join("docker-compose.yml"), "").unwrap();
        // the home directory's own file is still found from home itself
        assert_eq!(
            find_compose(&home, Some(&home)),
            Some(home.join("docker-compose.yml"))
        );
        std::fs::write(d.join("home/app/compose.yaml"), "").unwrap();
        assert_eq!(
            find_compose(&deep, Some(&home)),
            Some(d.join("home/app/compose.yaml"))
        );
        // above home is not searched
        let _ = std::fs::remove_file(d.join("home/app/compose.yaml"));
        let _ = std::fs::remove_file(home.join("docker-compose.yml"));
        std::fs::write(d.join("compose.yaml"), "").unwrap();
        assert_eq!(find_compose(&deep, Some(&home)), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn parses_ps_lines_and_arrays() {
        let line = r#"{"Name":"app-web-1","Service":"web","State":"running","Status":"Up 2 hours","Image":"nginx","Publishers":[{"URL":"0.0.0.0","TargetPort":80,"PublishedPort":8080,"Protocol":"tcp"},{"URL":"::","TargetPort":80,"PublishedPort":8080,"Protocol":"tcp"},{"URL":"","TargetPort":443,"PublishedPort":0,"Protocol":"tcp"}]}
{"Name":"app-db-1","Service":"db","State":"exited","Status":"Exited (0) 3 minutes ago","Image":"postgres","Publishers":[]}"#;
        let list = parse_ps(line);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].service, "web");
        assert_eq!(list[0].ports, ["8080→80"]);
        assert!(list[0].running() && !list[1].running());
        assert_eq!(list[0].uptime(), "2 hours");
        assert_eq!(list[1].uptime(), "Exited (0) 3 minutes ago");
        assert_eq!((list[0].health, list[1].health), (None, None));
        // older Compose: one array; exited ones sort after running ones
        let array = format!("[{}]", line.replace('\n', ","));
        let again = parse_ps(&array);
        assert_eq!(again, list);
        assert!(parse_ps("").is_empty());
    }

    #[test]
    fn health_and_uptime_split_out_of_the_status() {
        let ps = r#"{"Service":"db","State":"running","Status":"Up 5 hours (healthy)","Publishers":[{"TargetPort":53,"PublishedPort":5353,"Protocol":"udp"},{"TargetPort":3306,"PublishedPort":3306,"Protocol":"tcp"}]}
{"Service":"mail","State":"running","Status":"Up About a minute (health: starting)"}
{"Service":"api","State":"running","Status":"Up 2 minutes","Health":"unhealthy"}"#;
        let list = parse_ps(ps);
        let shown: Vec<(&str, String, Option<Health>)> = list
            .iter()
            .map(|c| (c.service.as_str(), c.uptime(), c.health))
            .collect();
        assert_eq!(
            shown,
            [
                ("api", "2 minutes".into(), Some(Health::Unhealthy)),
                ("db", "5 hours".into(), Some(Health::Healthy)),
                ("mail", "about a minute".into(), Some(Health::Starting)),
            ]
        );
        // published ports in port order, only non-TCP ones say so
        assert_eq!(list[1].ports, ["3306→3306", "5353→53/udp"]);
    }
}
